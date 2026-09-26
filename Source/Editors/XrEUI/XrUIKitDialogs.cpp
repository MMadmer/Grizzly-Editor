#define IMGUI_DEFINE_MATH_OPERATORS
#include "stdafx.h"
#include "imgui_internal.h"
#include "XrUIKitInternal.h"
#include <list>

using namespace XUI::Internal;

// ===================================================================================================
// Dialogs: a modal pinned to the editor's own window, a title row, a body that scrolls and a footer
// that stays put. The height fits what the dialog holds when it opens and then stays: a running job
// never makes it jump. Centred on open and whenever the work area changes, unless the author moved it.
// ===================================================================================================
namespace
{
struct Dialog
{
	xr_string name;
	ImGuiID popup = 0;
	ImGuiWindow* window = nullptr;
	int last_frame = -10;
	int shown = 0;
	bool moved = false;
	ImVec2 work_pos, work_size;
	float desired_h = 0.f;
	float shown_h = 0.f, max_h = 0.f;	// the height this frame was given, and the most it may take
	float body_content = 0.f;
	float body_bottom = 0.f;	// where the body ended this frame, px; 0 before it did
	float footer_w = 0.f, footer_w_cur = 0.f;
	float footer_x = 0.f, footer_y = 0.f;
	bool footer_used = true;
	bool footer_now = false;
	bool in_body = false;
	bool in_footer = false;
	bool danger = false, danger_cur = false;
	bool keys_ok = false;
	bool close = false;
	bool close_now = false;
	bool* open = nullptr;
};

xr_map<ImGuiID, Dialog> s_Dialogs;
xr_vector<Dialog*> s_Stack;

constexpr float kTitle = 36.f;
constexpr float kFooter = 52.f;
// the frames a dialog measures its height on; one that is not the final layout is never shown (EndDialog)
constexpr int kMeasureFrames = 2;

const char* NameOf(const char* title_id)
{
	const char* hash = strstr(title_id, "###");
	return hash ? hash + 3 : title_id;
}

const char* TitleEnd(const char* title_id)
{
	const char* hash = strstr(title_id, "###");
	return hash ? hash : LabelEnd(title_id);
}

// `refocus` only inside a frame: between frames nothing may move the focus
void ClosePopupById(ImGuiID popup, bool refocus)
{
	ImGuiContext& g = *GImGui;
	for (int i = 0; i < g.OpenPopupStack.Size; ++i)
		if (g.OpenPopupStack[i].PopupId == popup)
		{
			ImGui::ClosePopupToLevel(i, refocus);
			return;
		}
}

// BeginPopupModal keyed by an id instead of a name in the current window, pinned to the main viewport
bool BeginModal(ImGuiID popup, const char* name, ImGuiWindowFlags flags)
{
	ImGuiContext& g = *GImGui;
	if (!ImGui::IsPopupOpen(popup, ImGuiPopupFlags_None))
	{
		g.NextWindowData.ClearFlags();
		return false;
	}
	ImGui::SetNextWindowViewport(ImGui::GetMainViewport()->ID);
	flags |= ImGuiWindowFlags_Popup | ImGuiWindowFlags_Modal | ImGuiWindowFlags_NoCollapse | ImGuiWindowFlags_NoDocking;
	if (!ImGui::Begin(name, nullptr, flags))
	{
		ImGui::EndPopup();
		return false;
	}
	return true;
}

float WidthPt(XUI::DialogSize size, float work_w_pt)
{
	switch (size)
	{
	case XUI::DialogSize::S:	return 400.f;
	case XUI::DialogSize::L:	return 760.f;
	case XUI::DialogSize::XL:	return ImMin(1040.f, work_w_pt * .9f);
	default:					return 560.f;
	}
}

Dialog* Top()
{
	return s_Stack.empty() ? nullptr : s_Stack.back();
}

bool BeginDialogEx(const char* title_id, bool* open, XUI::DialogSize size, bool resizable, ImGuiID fixed, Dialog* parent)
{
	ImGuiContext& g = *GImGui;
	// a modal is a window of its own: a hidden or collapsed window that raises one does not hide it
	if (!title_id)
		return false;
	const ImGuiID popup = fixed ? fixed : g.CurrentWindow->GetID(title_id);
	Dialog& d = s_Dialogs[popup];
	if (d.last_frame < g.FrameCount - 1)
	{
		// not drawn last frame: this is an opening
		d.shown = 0;
		d.moved = false;
		d.desired_h = 0.f;
		d.footer_w = 0.f;
		d.footer_used = true;
		d.danger = false;
		d.close_now = false;
	}
	d.popup = popup;
	d.name = NameOf(title_id);
	d.open = open;

	const bool is_open = ImGui::IsPopupOpen(popup, ImGuiPopupFlags_None);
	bool keep = open ? *open : true;
	if (d.close)
	{
		keep = false;
		d.close = false;
	}
	if (!keep)
	{
		if (is_open)
			ClosePopupById(popup, true);
		if (open)
			*open = false;
		return false;
	}
	if (!is_open)
	{
		ImGui::OpenPopupEx(popup, ImGuiPopupFlags_None);
		d.shown = 0;
		d.moved = false;
		d.desired_h = 0.f;
		d.footer_w = 0.f;
	}

	const ImGuiViewport* vp = ImGui::GetMainViewport();
	float width = ImMin(XUI::Pt(WidthPt(size, vp->WorkSize.x / XUI::Dpi())), vp->WorkSize.x - XUI::Pt(32.f));
	float max_h = vp->WorkSize.y - XUI::Pt(48.f);
	if (parent && parent->window)
	{
		width = ImMin(width, parent->window->Size.x - XUI::Pt(32.f));
		max_h = ImMin(max_h, parent->window->Size.y - XUI::Pt(kTitle) - XUI::Pt(16.f));
	}
	width = ImMax(width, XUI::Pt(200.f));
	max_h = ImMax(max_h, XUI::Pt(kTitle + kFooter + 40.f));
	const float height = d.desired_h > 0.f ? ImMin(d.desired_h, max_h) : max_h;
	d.max_h = max_h;

	const bool area_changed = d.work_pos.x != vp->WorkPos.x || d.work_pos.y != vp->WorkPos.y ||
		d.work_size.x != vp->WorkSize.x || d.work_size.y != vp->WorkSize.y;
	d.work_pos = vp->WorkPos;
	d.work_size = vp->WorkSize;

	if (parent && parent->window)
		ImGui::SetNextWindowPos(ImVec2(parent->window->Pos.x + parent->window->Size.x * .5f, parent->window->Pos.y + XUI::Pt(kTitle)),
			ImGuiCond_Always, ImVec2(.5f, 0.f));
	else if (!d.moved || !d.window)
		ImGui::SetNextWindowPos(vp->WorkPos + vp->WorkSize * .5f, ImGuiCond_Always, ImVec2(.5f, .5f));
	else
	{
		// moved by the author: it stays where it was put, inside the window
		const ImVec2 size_now = d.window->Size;
		const ImVec2 inside(ImClamp(d.window->Pos.x, vp->WorkPos.x, ImMax(vp->WorkPos.x, vp->WorkPos.x + vp->WorkSize.x - size_now.x)),
			ImClamp(d.window->Pos.y, vp->WorkPos.y, ImMax(vp->WorkPos.y, vp->WorkPos.y + vp->WorkSize.y - size_now.y)));
		if (inside.x != d.window->Pos.x || inside.y != d.window->Pos.y)
			ImGui::SetNextWindowPos(inside, ImGuiCond_Always);
	}
	// a resizable one too takes the measured height, and is the author's from its first shown frame
	if (!resizable || d.shown <= kMeasureFrames || area_changed)
		ImGui::SetNextWindowSize(ImVec2(width, height), ImGuiCond_Always);
	if (resizable)
		ImGui::SetNextWindowSizeConstraints(PtV(240.f, 160.f), ImVec2(vp->WorkSize.x - XUI::Pt(32.f), max_h));

	ImGuiWindowFlags flags = ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoCollapse | ImGuiWindowFlags_NoSavedSettings |
		ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse | ImGuiWindowFlags_NoDocking;
	if (!resizable)
		flags |= ImGuiWindowFlags_NoResize;
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0.f, 0.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowRounding, XUI::Pt(parent ? 6.f : 8.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 1.f);
	ImGui::PushStyleColor(ImGuiCol_WindowBg, Col(Token::Overlay));
	ImGui::PushStyleColor(ImGuiCol_Border, Col(Token::BorderStrong));
	// the dialog window never scrolls, its body does: title, body and footer are placed from window->Pos, so a scroll
	// ImGui gave it (SetItemDefaultFocus on the first frame, a child's ScrollToRectEx) would only inflate ContentSize
	ImGui::SetNextWindowScroll(ImVec2(0.f, 0.f));
	// ImGui dims once, behind the top modal: a sheet dims its dialog and the editor alike
	const bool visible = BeginModal(popup, title_id, flags);
	ImGui::PopStyleColor(2);
	ImGui::PopStyleVar(3);
	if (!visible)
		return false;

	ImGuiWindow* window = g.CurrentWindow;
	d.window = window;
	d.last_frame = g.FrameCount;
	++d.shown;
	d.shown_h = window->Size.y;
	// never on the frame it opens, nor while nobody sees it: the key that opened it must not also answer it
	d.keys_ok = d.shown > 1 && !window->Hidden && ImGui::IsWindowFocused(ImGuiFocusedFlags_RootAndChildWindows) && !g.ActiveId &&
		!XUI::FromMcp();
	d.footer_now = false;
	d.body_bottom = 0.f;
	d.in_body = d.in_footer = false;
	d.danger_cur = false;
	d.footer_w_cur = 0.f;
	s_Stack.push_back(&d);

	// the title row: the name of the dialog, a close cross, and the grip to move it by
	const float title_h = XUI::Pt(kTitle);
	const ImRect title(window->Pos, ImVec2(window->Pos.x + window->Size.x, window->Pos.y + title_h));
	const float close_w = open ? XUI::Pt(40.f) : 0.f;
	if (!parent)
	{
		const ImGuiID drag = window->GetID("##dlg_title");
		const ImRect grip(title.Min, ImVec2(title.Max.x - close_w, title.Max.y));
		if (ImGui::ItemAdd(grip, drag, nullptr, ImGuiItemFlags_NoNav))
		{
			bool hovered = false, held = false;
			ImGui::ButtonBehavior(grip, drag, &hovered, &held);
			if (held && ImGui::IsMouseDragging(ImGuiMouseButton_Left, 0.f))
			{
				d.moved = true;
				ImGui::SetWindowPos(window, window->Pos + g.IO.MouseDelta, ImGuiCond_Always);
			}
		}
	}
	const xr_string caption = Str(title_id, TitleEnd(title_id));
	xr_string shown;
	const float room = title.GetWidth() - XUI::Pt(16.f) - (close_w > 0.f ? close_w : XUI::Pt(16.f));
	const bool cut = Ellipsize(XUI::Font::Title, caption.c_str(), room, XUI::Clip::End, shown);
	PaintText(window->DrawList, XUI::Font::Title, ImVec2(IM_ROUND(title.Min.x + XUI::Pt(16.f)), IM_ROUND(title.GetCenter().y - FontPx(XUI::Font::Title) * .5f)),
		Col(Token::Text), cut ? shown.c_str() : caption.c_str());
	if (cut)
		AuditAdd("truncated", caption.c_str(), title, shown.c_str());
	window->DrawList->AddLine(ImVec2(title.Min.x, title.Max.y - 1.f), ImVec2(title.Max.x, title.Max.y - 1.f), Col(Token::BorderSubtle));
	if (open)
	{
		ImGui::SetCursorScreenPos(ImVec2(title.Max.x - XUI::Pt(8.f) - XUI::Pt(24.f), title.GetCenter().y - XUI::Pt(12.f)));
		// a title bar item: never the focus a dialog opens with (that is its default button), never a Tab stop
		ImGui::PushItemFlag(ImGuiItemFlags_NoNavDefaultFocus | ImGuiItemFlags_NoTabStop, true);
		const bool closed = XUI::IconButton("##dlg_close", XUI::Icon::Close, "Close");
		ImGui::PopItemFlag();
		if (closed)
		{
			*open = false;
			d.close_now = true;
		}
	}
	ImGui::SetCursorScreenPos(ImVec2(window->Pos.x, title.Max.y));
	return true;
}

void EndBody(Dialog& d)
{
	ImGuiWindow* body = ImGui::GetCurrentWindow();
	d.body_content = body->DC.CursorMaxPos.y - body->DC.CursorStartPos.y + XUI::Pt(12.f) * 2.f;
	ImGui::EndChild();
	d.body_bottom = ImGui::GetItemRectMax().y;
	d.in_body = false;
}

bool DialogAlive(const xr_string& name)
{
	const int frame = ImGui::GetFrameCount();
	for (const auto& [id, d] : s_Dialogs)
		if (d.name == name && d.last_frame >= frame - 1)
			return true;
	return false;
}
}

bool XUI::Internal::InDialogScope()
{
	return !s_Stack.empty();
}

bool XUI::Internal::InDialogFooter()
{
	const Dialog* d = Top();
	return d && d->in_footer && GImGui->CurrentWindow == d->window;
}

void XUI::Internal::PlaceFooterButton(float width)
{
	Dialog* d = Top();
	if (!d || !d->in_footer)
		return;
	if (d->footer_w_cur > 0.f)
	{
		d->footer_x += Pt(8.f);
		d->footer_w_cur += Pt(8.f);
	}
	ImGui::SetCursorScreenPos(ImVec2(d->footer_x, d->footer_y));
	d->footer_x += width;
	d->footer_w_cur += width;
}

void XUI::Internal::MarkFooterDanger()
{
	if (Dialog* d = Top())
		d->danger = d->danger_cur = true;
}

void XUI::Internal::RecentreDialogs()
{
	for (auto& [id, d] : s_Dialogs)
		d.moved = false;
}

bool XUI::BeginDialog(cstr title_id, bool* open, DialogSize size, bool resizable)
{
	return BeginDialogEx(title_id, open, size, resizable, 0, nullptr);
}

bool XUI::BeginSheet(cstr title_id, bool* open, DialogSize size)
{
	// a sheet belongs to the dialog being drawn; outside one it is a dialog of its own
	return BeginDialogEx(title_id, open, size, false, 0, Top());
}

void XUI::BeginDialogBody()
{
	Dialog* d = Top();
	if (!d || d->in_body || d->in_footer)
		return;
	ImGuiWindow* window = d->window;
	const float footer = d->footer_used ? Pt(kFooter) : 0.f;
	const float top = window->DC.CursorPos.y;
	const float h = ImMax(Pt(24.f), window->Pos.y + window->Size.y - footer - top);
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, PtV(16.f, 12.f));
	ImGui::PushStyleColor(ImGuiCol_ChildBg, IM_COL32(0, 0, 0, 0));
	ImGui::BeginChild("##dlg_body", ImVec2(window->Size.x, h), ImGuiChildFlags_AlwaysUseWindowPadding);
	ImGui::PopStyleColor();
	ImGui::PopStyleVar();
	d->in_body = true;
}

bool XUI::BeginDialogFooter(cstr status, Tone tone)
{
	Dialog* d = Top();
	if (!d)
		return false;
	if (d->in_body)
		EndBody(*d);
	ImGuiWindow* window = d->window;
	const float h = Pt(kFooter);
	// right under the body; the cursor after it is ItemSpacing lower
	const float y = d->body_bottom > 0.f ? d->body_bottom : ImMax(window->DC.CursorPos.y, window->Pos.y + window->Size.y - h);
	window->DrawList->AddLine(ImVec2(window->Pos.x, y), ImVec2(window->Pos.x + window->Size.x, y), Col(Token::BorderSubtle));

	// status and extras on the left, in what the buttons leave
	if (status && *status)
	{
		const float icon = Pt(14.f);
		const float x = window->Pos.x + Pt(16.f);
		const float room = window->Size.x - Pt(32.f) - d->footer_w - Pt(16.f) - icon - Pt(6.f);
		const float cy = y + h * .5f;
		DrawIcon(window->DrawList, ToneIcon(tone), ImVec2(IM_ROUND(x), IM_ROUND(cy - icon * .5f)), 14.f, ToneColor(tone));
		xr_string shown;
		const bool cut = Ellipsize(Font::Body, status, ImMax(0.f, room), Clip::End, shown);
		const ImVec2 text_pos(IM_ROUND(x + icon + Pt(6.f)), IM_ROUND(cy - FontPx(Font::Body) * .5f));
		PaintText(window->DrawList, Font::Body, text_pos, tone == Tone::Muted ? Col(Token::TextSecondary) : ToneColor(tone),
			cut ? shown.c_str() : status);
		const ImRect bb(text_pos, text_pos + ImVec2(ImMax(0.f, room), FontPx(Font::Body)));
		if (cut)
		{
			AuditAdd("truncated", status, bb, shown.c_str());
			if (ImGui::ItemAdd(bb, 0) && ImGui::IsItemHovered(ImGuiHoveredFlags_ForTooltip))
				TooltipNow(status);
		}
	}
	d->footer_x = window->Pos.x + window->Size.x - Pt(16.f) - d->footer_w;
	d->footer_y = IM_ROUND(y + (h - Pt(28.f)) * .5f);
	d->in_footer = true;
	d->footer_now = true;
	ImGui::SetCursorScreenPos(ImVec2(d->footer_x, d->footer_y));
	return true;
}

void XUI::FooterBusy(cstr stage)
{
	Dialog* d = Top();
	if (!d || !d->in_footer)
		return;
	ImGuiWindow* window = d->window;
	const ImVec2 buttons = window->DC.CursorPos;
	const float h = Pt(28.f);
	const float cy = d->footer_y + h * .5f;
	const float text_y = IM_ROUND(cy - ImGui::GetTextLineHeight() * .5f);
	window->DC.CursorPos = ImVec2(window->Pos.x + Pt(16.f), text_y);
	Spinner();
	ImGui::SameLine(0.f, Pt(8.f));
	ImGui::SetCursorScreenPos(ImVec2(ImGui::GetCursorScreenPos().x, text_y));
	// the stage takes what the buttons leave (they start at footer_x until the first one is placed)
	const float room = d->footer_x - Pt(16.f) - ImGui::GetCursorScreenPos().x;
	TextEllipsis(stage && *stage ? stage : "Working...", ImMax(Em(3.f), room));
	// the buttons follow where the kit put the first one
	window->DC.CursorPos = buttons;
}

bool XUI::DialogButton(cstr label, Kind kind, bool is_default, bool on, cstr why)
{
	BtnSpec s;
	s.id = label;
	s.kind = kind;
	s.on = on;
	s.why = why;
	s.is_default = is_default;
	s.font = kind == Kind::Primary || kind == Kind::Danger ? Font::Strong : Font::Body;
	s.size = ImVec2(Pt(88.f), Pt(28.f));
	if (InDialogFooter())
	{
		if (kind == Kind::Danger)
			MarkFooterDanger();
		PlaceFooterButton(ButtonWidth(s));
	}
	bool pressed = ButtonEx(s);
	// the keyboard's focus opens on the default, the safe answer - not on the first button, which may be the Danger one
	if (is_default && on)
		ImGui::SetItemDefaultFocus();
	// Enter presses the default - never a Danger button, and never in a destructive dialog
	const Dialog* d = Top();
	if (!pressed && d && is_default && on && kind != Kind::Danger && !d->danger && d->keys_ok &&
		(ImGui::IsKeyPressed(ImGuiKey_Enter, false) || ImGui::IsKeyPressed(ImGuiKey_KeypadEnter, false)))
		pressed = true;
	return pressed;
}

namespace
{
void DrawSheetOf(Dialog& d);
}

void XUI::EndDialog()
{
	Dialog* d = Top();
	if (!d)
		return;
	if (d->in_body)
		EndBody(*d);
	if (d->in_footer)
	{
		d->in_footer = false;
		d->footer_w = d->footer_w_cur;
	}
	d->footer_used = d->footer_now;
	d->danger = d->danger_cur;

	// fitted to what it holds while it opens, fixed afterwards: the body scrolls from then on
	if (d->shown <= kMeasureFrames)
	{
		const float content = d->body_content > 0.f ? d->body_content
			: d->window->DC.CursorMaxPos.y - d->window->Pos.y - Pt(kTitle);
		d->desired_h = Pt(kTitle) + content + (d->footer_now ? Pt(kFooter) : 0.f) + 1.f;
		// Nobody sees a measuring frame that is not the final layout. ImGui hides a popup on the frame it opens; a later
		// one whose height the measure just changed is hidden here, before Render: the dialog is first shown at the
		// height it keeps, not a frame of it shrinking - to a person, a screenshot and ui_audit alike (Principle 8).
		if (d->shown > 1 && fabsf(ImMin(d->desired_h, d->max_h) - d->shown_h) > 1.5f)
		{
			d->window->Hidden = true;
			AuditHideWindow(d->window->Name);
		}
	}
	d->body_content = 0.f;

	// Esc cancels; so does Enter where the dialog would destroy something
	if (d->keys_ok && d->open && (ImGui::IsKeyPressed(ImGuiKey_Escape, false) ||
		(d->danger && (ImGui::IsKeyPressed(ImGuiKey_Enter, false) || ImGui::IsKeyPressed(ImGuiKey_KeypadEnter, false)))))
	{
		*d->open = false;
		d->close_now = true;
	}
	if (d->close_now)
	{
		d->close_now = false;
		ImGui::CloseCurrentPopup();
	}
	DrawSheetOf(*d);
	ImGui::EndPopup();
	s_Stack.pop_back();
}

void XUI::OpenDialogs(Strings& out)
{
	out.clear();
	const int frame = ImGui::GetCurrentContext() ? ImGui::GetFrameCount() : 0;
	for (const auto& [id, d] : s_Dialogs)
		if (d.last_frame >= frame - 1)
			out.push_back(d.name);
}

// ===================================================================================================
// Queued questions and notices: Confirm, UnsavedPrompt, Notice, Result
// ===================================================================================================
namespace
{
struct Queued
{
	enum Type { tConfirm, tUnsaved, tNotice, tResult };
	Type type = tNotice;
	xr_string title_id;
	xr_string headline, text, consequence, action;
	XUI::Strings items;
	u32 total = 0;
	bool destructive = false;
	XUI::Tone tone = XUI::Tone::Info;
	xr_vector<std::pair<xr_string, xr_string>> rows;
	XUI::Strings left_out, warnings;
	std::function<void(XUI::Answer)> then;
	std::function<void()> on;
	xr_string parent;
	bool open = true;
	bool done = false;
	XUI::Answer answer = XUI::Answer::Cancel;
};

std::list<Queued> s_Queue;

xr_string TitleId(const char* title, const char* id, const char* fallback)
{
	xr_string out = title && *title ? title : fallback;
	out += "###dlg.";
	out += id && *id ? id : "notice";
	return out;
}

void List(const XUI::Strings& items, u32 total, u32 limit)
{
	const u32 shown = ImMin(items.size(), limit);
	for (u32 i = 0; i < shown; ++i)
	{
		ImGui::Indent(XUI::Pt(8.f));
		XUI::TextEllipsis(items[i].c_str(), -1.f, XUI::Clip::Middle);
		ImGui::Unindent(XUI::Pt(8.f));
	}
	const u32 all = ImMax(total, items.size());
	if (all > shown)
	{
		char more[48];
		sprintf_s(more, "and %u more", all - shown);
		ImGui::Indent(XUI::Pt(8.f));
		ImGui::PushStyleColor(ImGuiCol_Text, Col(Token::TextMuted));
		ImGui::TextUnformatted(more);
		ImGui::PopStyleColor();
		ImGui::Unindent(XUI::Pt(8.f));
	}
}

void Headline(const xr_string& text)
{
	if (text.empty())
		return;
	XUI::FontScope strong(XUI::Font::Strong);
	ImGui::PushTextWrapPos(0.f);
	ImGui::TextUnformatted(text.c_str());
	ImGui::PopTextWrapPos();
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
}

void Reply(Queued& q, XUI::Answer answer)
{
	q.answer = answer;
	q.done = true;
	q.open = false;
	ImGui::CloseCurrentPopup();
}

void DrawQueued(Queued& q, Dialog* parent, ImGuiID fixed)
{
	const XUI::DialogSize size = q.type == Queued::tResult && (q.rows.size() > 6 || !q.warnings.empty()) ? XUI::DialogSize::L : XUI::DialogSize::M;
	if (!BeginDialogEx(q.title_id.c_str(), &q.open, size, false, fixed, parent))
	{
		if (!q.open && !q.done)
		{
			q.done = true;
			q.answer = XUI::Answer::Cancel;
		}
		return;
	}
	XUI::BeginDialogBody();
	switch (q.type)
	{
	case Queued::tConfirm:
		Headline(q.headline);
		List(q.items, q.total, 10);
		if (!q.consequence.empty() || q.destructive)
		{
			ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
			XUI::StatusText(XUI::Tone::Warning, q.consequence.empty() ? "Deleted for good - this cannot be undone." : q.consequence.c_str());
		}
		break;
	case Queued::tUnsaved:
		Headline(q.headline);
		if (q.items.size() > 1)
			List(q.items, 0, 10);
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
		XUI::StatusText(XUI::Tone::Muted, "Changes are lost if they are not saved.");
		break;
	case Queued::tNotice:
		XUI::StatusText(q.tone, q.text.c_str());
		break;
	case Queued::tResult:
		XUI::StatusText(q.tone, q.headline.c_str());
		if (!q.rows.empty() && ImGui::BeginTable("##rows", 2, ImGuiTableFlags_SizingStretchProp))
		{
			ImGui::TableSetupColumn("key", ImGuiTableColumnFlags_WidthFixed, XUI::Em(9.f));
			ImGui::TableSetupColumn("value", ImGuiTableColumnFlags_WidthStretch);
			for (const auto& [key, value] : q.rows)
			{
				ImGui::TableNextRow();
				ImGui::TableNextColumn();
				ImGui::PushStyleColor(ImGuiCol_Text, Col(Token::TextSecondary));
				XUI::TextEllipsis(key.c_str());
				ImGui::PopStyleColor();
				ImGui::TableNextColumn();
				XUI::TextEllipsis(value.c_str(), -1.f, XUI::Clip::Middle);
			}
			ImGui::EndTable();
		}
		if (!q.left_out.empty())
		{
			ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
			Headline("Not shipped");
			List(q.left_out, 0, 10);
		}
		if (!q.warnings.empty())
		{
			ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
			const u32 shown = ImMin(q.warnings.size(), 8u);
			for (u32 i = 0; i < shown; ++i)
				XUI::StatusText(XUI::Tone::Warning, q.warnings[i].c_str());
			if (q.warnings.size() > shown)
			{
				char more[48];
				sprintf_s(more, "and %u more warnings", q.warnings.size() - shown);
				ImGui::PushStyleColor(ImGuiCol_Text, Col(Token::TextMuted));
				ImGui::TextUnformatted(more);
				ImGui::PopStyleColor();
			}
		}
		break;
	}

	if (XUI::BeginDialogFooter())
	{
		switch (q.type)
		{
		case Queued::tConfirm:
		{
			const char* action = !q.action.empty() ? q.action.c_str() : q.destructive ? "Delete" : "OK";
			if (XUI::DialogButton(action, q.destructive ? XUI::Kind::Danger : XUI::Kind::Primary, !q.destructive))
				Reply(q, XUI::Answer::Yes);
			if (XUI::DialogButton("Cancel", XUI::Kind::Secondary, q.destructive))
				Reply(q, XUI::Answer::Cancel);
			break;
		}
		case Queued::tUnsaved:
			if (XUI::DialogButton(q.items.size() > 1 ? "Save All" : "Save", XUI::Kind::Primary, true))
				Reply(q, XUI::Answer::Yes);
			if (XUI::DialogButton("Don't Save", XUI::Kind::Secondary))
				Reply(q, XUI::Answer::No);
			if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
				Reply(q, XUI::Answer::Cancel);
			break;
		case Queued::tResult:
			if (!q.action.empty() && XUI::DialogButton(q.action.c_str(), XUI::Kind::Secondary) && q.on)
				q.on();
			if (XUI::DialogButton("OK", XUI::Kind::Primary, true))
				Reply(q, XUI::Answer::Yes);
			break;
		default:
			if (XUI::DialogButton("OK", XUI::Kind::Primary, true))
				Reply(q, XUI::Answer::Yes);
			break;
		}
	}
	XUI::EndDialog();
	if (!q.open && !q.done)
	{
		q.done = true;
		q.answer = XUI::Answer::Cancel;
	}
}

void DrawSheetOf(Dialog& d)
{
	for (Queued& q : s_Queue)
	{
		if (q.done || q.parent.empty() || q.parent != d.name)
			continue;
		DrawQueued(q, &d, 0);
		return;
	}
}

// re-enters the modals open now, so a dialog opened at the end of the frame nests above them
// instead of closing them (a popup opened at a lower level closes every one above it)
int EnterModalChain(ImGuiID popup)
{
	ImGuiContext& g = *GImGui;
	int level = -1;
	for (int i = 0; i < g.OpenPopupStack.Size && level < 0; ++i)
		if (g.OpenPopupStack[i].PopupId == popup)
			level = i;
	int target = 0;
	if (level >= 0)
		target = level;
	else
		for (int i = 0; i < g.OpenPopupStack.Size; ++i)
		{
			const ImGuiWindow* w = g.OpenPopupStack[i].Window;
			if (w && (w->Flags & ImGuiWindowFlags_Modal) && w->LastFrameActive == g.FrameCount)
				target = i + 1;
		}
	int depth = 0;
	for (int i = 0; i < target; ++i)
	{
		ImGuiWindow* w = g.OpenPopupStack[i].Window;
		if (!w || w->LastFrameActive != g.FrameCount)
			break;
		ImGui::Begin(w->Name, nullptr, w->Flags);
		++depth;
	}
	return depth;
}

void FinishAnswered()
{
	for (auto it = s_Queue.begin(); it != s_Queue.end();)
	{
		if (!it->done)
		{
			++it;
			continue;
		}
		Queued q = std::move(*it);
		it = s_Queue.erase(it);
		if (q.then)
			q.then(q.answer);
	}
}

void Log(XUI::Tone tone, const char* what, const char* text)
{
	const char* mark = tone == XUI::Tone::Error ? "!" : tone == XUI::Tone::Warning ? "~" : "*";
	Msg("%s %s%s%s", mark, what ? what : "", what && text && *text ? ": " : "", text ? text : "");
}
}

void XUI::Internal::DrawQueuedDialogs()
{
	FinishAnswered();
	// a sheet whose dialog has gone becomes a dialog of its own
	for (Queued& q : s_Queue)
		if (!q.parent.empty() && !DialogAlive(q.parent))
			q.parent.clear();
	for (Queued& q : s_Queue)
	{
		if (q.done || !q.parent.empty())
			continue;
		const ImGuiID popup = ImHashStr(q.title_id.c_str(), 0, 0x58554964);
		const int depth = EnterModalChain(popup);
		DrawQueued(q, nullptr, popup);
		for (int i = 0; i < depth; ++i)
			ImGui::End();
		break;
	}
	FinishAnswered();
}

bool XUI::CloseDialog(cstr name)
{
	if (!name || !*name)
		return false;
	xr_string full = name;
	if (strncmp(name, "dlg.", 4))
		full.insert(0, "dlg.");
	bool found = false;
	for (Queued& q : s_Queue)
		if (!q.done && !strcmp(NameOf(q.title_id.c_str()), full.c_str()))
		{
			q.done = true;
			q.open = false;
			q.answer = Answer::Cancel;
			found = true;
		}
	const int frame = ImGui::GetCurrentContext() ? ImGui::GetFrameCount() : 0;
	for (auto& [id, d] : s_Dialogs)
		if ((d.name == full || d.name == name) && d.last_frame >= frame - 1)
		{
			d.close = true;
			if (d.open)
				*d.open = false;
			ClosePopupById(d.popup, GImGui->WithinFrameScope);
			found = true;
		}
	return found;
}

void XUI::Confirm(const ConfirmSpec& spec, std::function<void(Answer)> then)
{
	const char* what = spec.headline ? spec.headline : spec.title ? spec.title : "Confirm";
	// the caller over MCP has decided: the action runs before the reply
	if (FromMcp())
	{
		Msg("* [mcp] %s -> Yes", what);
		if (then)
			then(Answer::Yes);
		return;
	}
	if (Unattended())
	{
		Msg("~ [dlg suppressed] %s  -> auto-answer: No", what);
		if (then)
			then(Answer::No);
		return;
	}
	Queued q;
	q.type = Queued::tConfirm;
	q.title_id = TitleId(spec.title, spec.id, "Confirm");
	q.headline = spec.headline ? spec.headline : "";
	q.items = spec.items;
	q.total = spec.total;
	q.consequence = spec.consequence ? spec.consequence : "";
	q.action = spec.action ? spec.action : "";
	q.destructive = spec.destructive;
	q.then = std::move(then);
	if (InDialogScope())
		q.parent = Top()->name;
	s_Queue.push_back(std::move(q));
}

void XUI::UnsavedPrompt(cstr id, const Strings& what, std::function<void(Answer)> then)
{
	xr_string list;
	for (const xr_string& item : what)
	{
		if (!list.empty())
			list += ", ";
		list += item;
	}
	if (FromMcp())
	{
		const Answer answer = McpUnsavedAnswer(what);
		Msg("* [mcp] unsaved changes (%s) -> %s", list.c_str(), answer == Answer::Yes ? "Save" : answer == Answer::No ? "Don't Save" : "Cancel");
		if (then)
			then(answer);
		return;
	}
	if (Unattended())
	{
		Msg("~ [dlg suppressed] Unsaved changes: %s  -> auto-answer: No", list.c_str());
		if (then)
			then(Answer::No);
		return;
	}
	Queued q;
	q.type = Queued::tUnsaved;
	q.title_id = TitleId("Unsaved Changes", id ? id : "unsaved", "Unsaved Changes");
	if (what.size() == 1)
	{
		q.headline = "Save changes to ";
		q.headline += what[0];
		q.headline += "?";
	}
	else
		q.headline = "Save changes to these items?";
	q.items = what;
	q.then = std::move(then);
	if (InDialogScope())
		q.parent = Top()->name;
	s_Queue.push_back(std::move(q));
}

void XUI::Notice(Tone tone, cstr title, cstr text)
{
	Log(tone, title, text);
	// nothing waits on a notice: an MCP caller or an unattended run has it in the log
	if (FromMcp() || Unattended())
		return;
	Queued q;
	q.type = Queued::tNotice;
	q.tone = tone;
	q.title_id = TitleId(title, "notice", tone == Tone::Error ? "Error" : tone == Tone::Warning ? "Warning" : "Information");
	q.text = text ? text : "";
	s_Queue.push_back(std::move(q));
}

void XUI::Result(cstr id, cstr title, Tone tone, cstr headline, const xr_vector<std::pair<xr_string, xr_string>>& rows,
	const Strings& left_out, const Strings& warnings, cstr action, std::function<void()> on)
{
	Log(tone, title, headline);
	for (const xr_string& warning : warnings)
		Msg("~ %s", warning.c_str());
	// it opens only for runs started from the UI
	if (FromMcp() || Unattended())
		return;
	Queued q;
	q.type = Queued::tResult;
	q.tone = tone;
	q.title_id = TitleId(title, id ? id : "result", "Result");
	q.headline = headline ? headline : "";
	q.rows = rows;
	q.left_out = left_out;
	q.warnings = warnings;
	q.action = action ? action : "";
	q.on = std::move(on);
	s_Queue.push_back(std::move(q));
}

// ===================================================================================================
// Toasts: bottom right, stacked, gone after a few seconds - they do not fade, only the spinner moves
// ===================================================================================================
namespace
{
struct ToastItem
{
	XUI::Tone tone = XUI::Tone::Info;
	xr_string text;
	double born = -1.0;
	u32 id = 0;
};
xr_vector<ToastItem> s_Toasts;
u32 s_ToastSerial = 0;
}

void XUI::Toast(Tone tone, cstr text)
{
	if (!text || !*text)
		return;
	Log(tone, nullptr, text);
	ToastItem& t = s_Toasts.emplace_back();
	t.tone = tone;
	t.text = text;
	t.id = ++s_ToastSerial;
	while (s_Toasts.size() > 4)
		s_Toasts.erase(s_Toasts.begin());
}

void XUI::Internal::DrawToasts()
{
	if (s_Toasts.empty())
		return;
	const double now = ImGui::GetTime();
	for (auto it = s_Toasts.begin(); it != s_Toasts.end();)
	{
		if (it->born < 0.0)
			it->born = now;
		const double life = it->tone == Tone::Error ? 10.0 : 6.0;
		it = now - it->born > life ? s_Toasts.erase(it) : it + 1;
	}
	if (s_Toasts.empty())
		return;

	const ImGuiViewport* vp = ImGui::GetMainViewport();
	ImGui::SetNextWindowPos(ImVec2(vp->WorkPos.x + vp->WorkSize.x - Pt(16.f), vp->WorkPos.y + vp->WorkSize.y - Pt(16.f)), ImGuiCond_Always, ImVec2(1.f, 1.f));
	ImGui::SetNextWindowViewport(vp->ID);
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0.f, 0.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 0.f);
	ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, PtV(6.f, 6.f));
	ImGui::PushStyleColor(ImGuiCol_WindowBg, IM_COL32(0, 0, 0, 0));
	const ImGuiWindowFlags flags = ImGuiWindowFlags_NoDecoration | ImGuiWindowFlags_NoSavedSettings | ImGuiWindowFlags_NoFocusOnAppearing |
		ImGuiWindowFlags_NoNav | ImGuiWindowFlags_NoDocking | ImGuiWindowFlags_AlwaysAutoResize | ImGuiWindowFlags_NoMove;
	if (ImGui::Begin("##xui.toasts", nullptr, flags))
	{
		ImGui::BringWindowToDisplayFront(ImGui::GetCurrentWindow());
		const float width = Pt(360.f);
		int remove = -1;
		for (int i = 0; i < int(s_Toasts.size()); ++i)
		{
			const ToastItem& t = s_Toasts[i];
			ImGui::PushID(int(t.id));
			ImGui::PushStyleColor(ImGuiCol_ChildBg, Col(Token::Overlay));
			ImGui::PushStyleColor(ImGuiCol_Border, Col(Token::BorderStrong));
			ImGui::PushStyleVar(ImGuiStyleVar_ChildRounding, Pt(6.f));
			ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, PtV(12.f, 10.f));
			if (ImGui::BeginChild("##toast", ImVec2(width, 0.f), ImGuiChildFlags_Borders | ImGuiChildFlags_AutoResizeY | ImGuiChildFlags_AlwaysUseWindowPadding))
			{
				ImGuiWindow* window = ImGui::GetCurrentWindow();
				const ImVec2 pos = window->DC.CursorPos;
				const float close = Pt(20.f);
				ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, PtV(8.f, 4.f));
				ImGui::PushTextWrapPos(window->WorkRect.Max.x - close - Pt(4.f) - window->Pos.x + window->Scroll.x);
				ImGui::Indent(Pt(22.f));
				DrawIcon(window->DrawList, ToneIcon(t.tone), ImVec2(pos.x, pos.y + (ImGui::GetTextLineHeight() - Pt(16.f)) * .5f), 16.f, ToneColor(t.tone));
				ImGui::TextUnformatted(t.text.c_str());
				ImGui::Unindent(Pt(22.f));
				ImGui::PopTextWrapPos();
				ImGui::PopStyleVar();
				const ImVec2 backup = window->DC.CursorPos;
				ImGui::SetCursorScreenPos(ImVec2(window->InnerRect.Max.x - Pt(12.f) - close + Pt(4.f), pos.y - Pt(2.f)));
				BtnSpec s;
				s.id = "##dismiss";
				s.text = "";
				s.icon = Icon::Close;
				s.icon_pt = 12.f;
				s.kind = Kind::Ghost;
				s.tip = "Dismiss";
				s.size = ImVec2(close, close);
				s.layout = false;
				if (ButtonEx(s))
					remove = i;
				window->DC.CursorPos = backup;
			}
			ImGui::EndChild();
			ImGui::PopStyleVar(2);
			ImGui::PopStyleColor(2);
			ImGui::PopID();
		}
		if (remove >= 0)
			s_Toasts.erase(s_Toasts.begin() + remove);
	}
	ImGui::End();
	ImGui::PopStyleColor();
	ImGui::PopStyleVar(3);
}
