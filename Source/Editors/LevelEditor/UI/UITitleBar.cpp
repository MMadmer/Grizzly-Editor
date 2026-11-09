#include "stdafx.h"
#include "UITitleBar.h"
#include "..\..\XrECore\Editor\EditorTitleBar.h"
#include "..\..\XrECore\Editor\EditorProject.h"
#include "..\..\XrECore\Editor\GrizzlyMCP.h"
#include "..\..\XrEUI\XrUITheme.h"
#include "..\..\Public\grizzly_resource.h"
#include "..\..\XrECore\Editor\EDX11Utils.h"
#include "..\..\XrECore\Editor\Nq\NqUtil.h"

// The main menu bar doubles as the window caption (see EditorTitleBar.h for the Win32 half). Everything the bar
// draws remembers its rectangle, and the hit test replays those rectangles for the WndProc: menus, tabs and
// buttons are client area, the leftover band drags the window.
namespace
{
struct SBand
{
	// client px, valid for the frame that drew them
	float	bar_x0, bar_y0, bar_x1, bar_y1;
	float	menus_x0;			// menus and tabs occupy [menus_x0 .. menus_end_x]
	float	menus_end_x;
	float	buttons_x0;			// buttons occupy [buttons_x0 .. bar_x1]
	bool	valid;
};
static SBand		s_Band = {};
static ImTextureID	s_Icon = 0;
static int			s_IconW = 0, s_IconH = 0;
static xr_string	s_OsTitle;

float Round(float v)
{
	return floorf(v + .5f);
}

ImU32 Col(GrizzlyTheme::ColorToken token, float alpha = 1.f)
{
	return GrizzlyTheme::U32(token, alpha);
}

// The window icon as an ImGui texture: the top-down DIB is already the
// B8G8R8A8 layout the DX11 helper uploads verbatim.
void LoadIconTexture()
{
	const int size = 32;
	HICON icon = (HICON)::LoadImageW(::GetModuleHandleW(0), MAKEINTRESOURCEW(IDI_GRIZZLY_EDITOR),
		IMAGE_ICON, size, size, 0);
	if (!icon)					return;

	HDC dc = ::CreateCompatibleDC(0);
	BITMAPINFO bi = {};
	bi.bmiHeader.biSize = sizeof(bi.bmiHeader);
	bi.bmiHeader.biWidth = size;
	bi.bmiHeader.biHeight = -size;			// top-down
	bi.bmiHeader.biPlanes = 1;
	bi.bmiHeader.biBitCount = 32;
	bi.bmiHeader.biCompression = BI_RGB;
	void* bits = 0;
	if (HBITMAP bmp = ::CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &bits, 0, 0))
	{
		HGDIOBJ old = ::SelectObject(dc, bmp);
		// the DIB starts undefined and DrawIconEx alpha-blends: clear first
		memset(bits, 0, size_t(size) * size * 4);
		::DrawIconEx(dc, 0, 0, icon, size, size, 0, 0, DI_NORMAL);
		::SelectObject(dc, old);

		if (ID3D11ShaderResourceView* view = DX11TextureFromPixels((const u32*)bits, size, size))
		{
			s_Icon = static_cast<ImTextureID>(view);
			s_IconW = s_IconH = size;
		}
		::DeleteObject(bmp);
	}
	::DeleteDC(dc);
	::DestroyIcon(icon);
}

// 46 x 32 pt, frameless: a whisper of a fill on hover, red only under the close cross
enum EButton { btnMin, btnMax, btnClose };

bool WindowButton(EButton kind, float x, float y, bool busy)
{
	char id[8];
	sprintf_s(id, "##wb%d", int(kind));
	const ImVec2 size(XUI::Pt(46.f), XUI::Pt(32.f));
	ImGui::SetCursorScreenPos(ImVec2(x, y));
	ImGui::BeginDisabled(busy);
	const bool pressed = ImGui::InvisibleButton(id, size);
	const bool hovered = ImGui::IsItemHovered();
	const bool held = ImGui::IsItemActive();
	ImGui::EndDisabled();
	const ImVec2 a = ImGui::GetItemRectMin();
	const ImVec2 b = ImGui::GetItemRectMax();
	ImDrawList* dl = ImGui::GetWindowDrawList();
	if (hovered && !busy)
		dl->AddRectFilled(a, b, kind == btnClose ? Col(held ? GrizzlyTheme::ColorToken::DangerPressed : GrizzlyTheme::ColorToken::DangerHovered)
			: Col(held ? GrizzlyTheme::ColorToken::Pressed : GrizzlyTheme::ColorToken::Hover));
	const XUI::Icon icon = kind == btnMin ? XUI::Icon::Minimize :
		kind == btnClose ? XUI::Icon::Close : ::IsZoomed(EDevice->m_hWnd) ? XUI::Icon::Restore : XUI::Icon::Maximize;
	const ImU32 col = busy ? Col(GrizzlyTheme::ColorToken::TextDisabled) :
		Col(hovered ? GrizzlyTheme::ColorToken::Text : GrizzlyTheme::ColorToken::TextSecondary);
	const float glyph = XUI::Pt(12.f);
	XUI::DrawIcon(dl, icon, ImVec2(Round((a.x + b.x - glyph) * .5f), Round((a.y + b.y - glyph) * .5f)), 12.f, col);
	const char* tips[] = { "Minimize", "Maximize", "Close" };
	if (!busy)
		XUI::Tip(kind == btnMax && ::IsZoomed(EDevice->m_hWnd) ? "Restore" : tips[kind]);
	return pressed && !busy;
}

int HitTest(int x, int y)
{
	if (!s_Band.valid)								return 0;
	if (y < int(s_Band.bar_y0) || y >= int(s_Band.bar_y1))	return 0;
	if (x < int(s_Band.bar_x0) || x >= int(s_Band.bar_x1))	return 0;
	// menus, tabs and buttons take the mouse; the rest of the band drags the window
	if (x >= int(s_Band.menus_x0) && x < int(s_Band.menus_end_x))	return HTCLIENT;
	if (x >= int(s_Band.buttons_x0))				return HTCLIENT;
	return HTCAPTION;
}

// "<project> - Grizzly Editor" on the taskbar and in Alt+Tab
void SyncOsTitle()
{
	xr_string title = "Grizzly Editor";
	if (EditorProject::Active())
	{
		title = EditorProject::Name();
		title += " - Grizzly Editor";
	}
	if (title == s_OsTitle)
		return;
	s_OsTitle = title;
	wchar_t wide[512] = {};
	::MultiByteToWideChar(CP_UTF8, 0, title.c_str(), -1, wide, 512);
	::SetWindowTextW(EDevice->m_hWnd, wide);
}
} // namespace

namespace UITitleBar
{
void Install()
{
	EditorTitleBar::SetHitTest(&HitTest);
}

bool Begin()
{
	if (!XUI::Shell::BeginCaption())
	{
		// a frame that does not draw the caption leaves no band to drag
		s_Band.valid = false;
		return false;
	}
	const ImVec2 wp = ImGui::GetWindowPos();
	const ImVec2 ws = ImGui::GetWindowSize();
	s_Band.bar_x0 = wp.x;
	s_Band.bar_y0 = wp.y;
	s_Band.bar_x1 = wp.x + ws.x;
	s_Band.bar_y1 = wp.y + ws.y;
	s_Band.buttons_x0 = s_Band.bar_x1 - 3.f * XUI::Pt(46.f);
	return true;
}

void End()
{
	XUI::Shell::EndCaption();
	SyncOsTitle();
}

void DrawLeft()
{
	const float h = s_Band.bar_y1 - s_Band.bar_y0;
	// lazy: Install() runs before the renderer exists, a texture cannot
	static bool icon_tried = false;
	if (!icon_tried) { icon_tried = true; LoadIconTexture(); }
	const ImVec2 p = ImGui::GetCursorScreenPos();
	const float side = XUI::Pt(20.f);
	const float x0 = s_Band.bar_x0 + XUI::Pt(10.f);
	if (s_Icon)
		ImGui::GetWindowDrawList()->AddImage(s_Icon, ImVec2(Round(x0), Round(s_Band.bar_y0 + (h - side) * .5f)),
			ImVec2(Round(x0 + side), Round(s_Band.bar_y0 + (h + side) * .5f)));
	ImGui::SetCursorScreenPos(ImVec2(x0 + side + XUI::Pt(6.f), p.y));
	s_Band.menus_x0 = ImGui::GetCursorScreenPos().x;
	s_Band.menus_end_x = s_Band.menus_x0;
}

float TabsMaxX()
{
	// a drag band of at least 48 pt stays before the window buttons
	return s_Band.buttons_x0 - XUI::Pt(48.f);
}

void DrawRight(bool busy)
{
	const float h = s_Band.bar_y1 - s_Band.bar_y0;
	s_Band.menus_end_x = ImGui::GetCursorScreenPos().x;

	// the project's name in the band, the first thing to go when the tabs need the room
	if (EditorProject::Active())
	{
		const char* name = EditorProject::Name();
		const ImVec2 size = ImGui::CalcTextSize(name);
		const float right = s_Band.buttons_x0 - XUI::Pt(12.f);
		if (right - size.x >= s_Band.menus_end_x + XUI::Pt(48.f))
			ImGui::GetWindowDrawList()->AddText(ImVec2(Round(right - size.x), Round(s_Band.bar_y0 + (h - size.y) * .5f)),
				Col(GrizzlyTheme::ColorToken::TextMuted), name);
	}

	// the window buttons, flush with the window's edge
	ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(0.f, 0.f));
	const float bw = XUI::Pt(46.f);
	if (WindowButton(btnMin, s_Band.buttons_x0, s_Band.bar_y0, busy))
		::ShowWindow(EDevice->m_hWnd, SW_MINIMIZE);
	if (WindowButton(btnMax, s_Band.buttons_x0 + bw, s_Band.bar_y0, busy))
		::ShowWindow(EDevice->m_hWnd, ::IsZoomed(EDevice->m_hWnd) ? SW_RESTORE : SW_MAXIMIZE);
	if (WindowButton(btnClose, s_Band.buttons_x0 + 2.f * bw, s_Band.bar_y0, busy))
		::PostMessageW(EDevice->m_hWnd, WM_CLOSE, 0, 0);	// the exit prompt path
	ImGui::PopStyleVar();

	s_Band.valid = true;
}

void McpInfo(xr_string& out)
{
	char tmp[640];
	sprintf_s(tmp,
		"{\"ok\":true,\"enabled\":%s,\"band\":[%d,%d,%d,%d],\"menus_end\":%d,\"buttons_x\":%d,"
		"\"maximized\":%s,\"icon\":%s,\"caption_h\":%d,\"dpi\":%.2f,\"folded\":%s,\"interactive\":[[%d,%d],[%d,%d]],\"tabs\":[",
		EditorTitleBar::Enabled() ? "true" : "false",
		int(s_Band.bar_x0), int(s_Band.bar_y0), int(s_Band.bar_x1), int(s_Band.bar_y1),
		int(s_Band.menus_end_x), int(s_Band.buttons_x0),
		::IsZoomed(EDevice->m_hWnd) ? "true" : "false",
		s_Icon ? "true" : "false",
		int(s_Band.bar_y1 - s_Band.bar_y0), XUI::Dpi(), UIMainMenuForm::Folded() ? "true" : "false",
		int(s_Band.menus_x0), int(s_Band.menus_end_x), int(s_Band.buttons_x0), int(s_Band.bar_x1));
	out = tmp;
	xr_vector<ImVec4> rects;
	xr_vector<xr_string> ids;
	XUI::Shell::TabRects(rects, &ids);
	for (u32 i = 0; i < rects.size(); ++i)
	{
		// a tab's id carries its file's path: escaped like any other string
		out += i ? ",{\"id\":" : "{\"id\":";
		NqUtil::JsonString(out, ids[i]);
		char row[128];
		sprintf_s(row, ",\"rect\":[%d,%d,%d,%d]}", int(rects[i].x), int(rects[i].y), int(rects[i].z), int(rects[i].w));
		out += row;
	}
	out += "]}";
}
} // namespace UITitleBar
