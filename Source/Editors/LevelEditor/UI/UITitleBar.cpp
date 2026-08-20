#include "stdafx.h"
#include "UITitleBar.h"
#include "..\..\XrECore\Editor\EditorTitleBar.h"
#include "..\..\XrECore\Editor\EditorProject.h"
#include "..\..\XrECore\Editor\XFinedMCP.h"
#include "..\..\XrEUI\XrUITheme.h"
#include "..\..\Public\xfined_resource.h"
#include "..\..\XrECore\Editor\EDX11Utils.h"

// The main menu bar doubles as the window caption (see EditorTitleBar.h for
// the Win32 half). Everything the bar draws remembers its rectangle, and the
// hit-test replays those rectangles for the WndProc: buttons and menus are
// client area, the leftover band drags the window.
namespace
{
struct SBand
{
	// client px, valid for the frame that drew them
	float	bar_x0, bar_y0, bar_x1, bar_y1;
	float	menus_end_x;		// menus occupy [bar_x0 .. menus_end_x]
	float	buttons_x0;			// buttons occupy [buttons_x0 .. bar_x1]
	bool	valid;
};
static SBand		s_Band = {};
static ImTextureID	s_Icon = 0;
static int			s_IconW = 0, s_IconH = 0;

ImU32 ThemeColor(XFinedTheme::ColorToken token, float alpha)
{
	const u32 rgb = XFinedTheme::Rgb(token);
	return IM_COL32((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF, int(alpha * 255.f));
}

// The window icon as an ImGui texture: the top-down DIB is already the
// B8G8R8A8 layout the DX11 helper uploads verbatim.
void LoadIconTexture()
{
	const int size = 32;
	HICON icon = (HICON)::LoadImageW(::GetModuleHandleW(0), MAKEINTRESOURCEW(IDI_XFINED_EDITOR),
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

// Thin accent strokes, Unreal-style: nothing but the glyph itself, a whisper
// of a background on hover, red only under the close cross.
enum EButton { btnMin, btnMax, btnClose };

bool WindowButton(EButton kind, float side)
{
	char id[8];
	sprintf_s(id, "##wb%d", int(kind));
	ImGui::PushStyleColor(ImGuiCol_Button, IM_COL32(0, 0, 0, 0));
	ImGui::PushStyleColor(ImGuiCol_ButtonHovered,
		kind == btnClose ? IM_COL32(196, 43, 28, 170) : IM_COL32(255, 255, 255, 16));
	ImGui::PushStyleColor(ImGuiCol_ButtonActive,
		kind == btnClose ? IM_COL32(196, 43, 28, 220) : IM_COL32(255, 255, 255, 28));
	ImGui::PushStyleVar(ImGuiStyleVar_FrameRounding, 0.f);
	const bool pressed = ImGui::Button(id, ImVec2(side * 1.55f, side));
	ImGui::PopStyleVar();
	ImGui::PopStyleColor(3);

	const ImVec2 a = ImGui::GetItemRectMin();
	const ImVec2 b = ImGui::GetItemRectMax();
	const ImVec2 c((a.x + b.x) * 0.5f, (a.y + b.y) * 0.5f);
	const bool hovered = ImGui::IsItemHovered();
	// the cross reads best white on its red plate; everything else stays accent
	// idle strokes are the theme accent at full strength - the muted purple washes
	// out to grey on the dark bar at anything less; hover answers with the bright one
	const ImU32 col = (kind == btnClose && hovered)
		? IM_COL32(255, 255, 255, 255)
		: ThemeColor(hovered ? XFinedTheme::ColorToken::AccentActive : XFinedTheme::ColorToken::Accent, 1.f);
	ImDrawList* dl = ImGui::GetWindowDrawList();
	const float r = side * 0.20f;			// glyph half-extent
	const float t = std::max(1.f, side / 16.f);// stroke, DPI-proportional

	switch (kind)
	{
	case btnMin:
		dl->AddLine(ImVec2(c.x - r, c.y), ImVec2(c.x + r, c.y), col, t);
		break;
	case btnMax:
		if (::IsZoomed(EDevice->m_hWnd))
		{
			// restore: the back pane peeks out behind the front one
			const float o = r * 0.45f;
			dl->AddRect(ImVec2(c.x - r + o, c.y - r - o + 2.f * o), ImVec2(c.x + r, c.y + r - 2.f * o + o), col, 1.f, 0, t);
			dl->AddRect(ImVec2(c.x - r, c.y - r + o), ImVec2(c.x + r - o, c.y + r), col, 1.f, 0, t);
		}
		else
			dl->AddRect(ImVec2(c.x - r, c.y - r), ImVec2(c.x + r, c.y + r), col, 1.f, 0, t);
		break;
	case btnClose:
		dl->AddLine(ImVec2(c.x - r, c.y - r), ImVec2(c.x + r, c.y + r), col, t);
		dl->AddLine(ImVec2(c.x - r, c.y + r), ImVec2(c.x + r, c.y - r), col, t);
		break;
	}
	return pressed;
}

int HitTest(int x, int y)
{
	if (!s_Band.valid)								return 0;
	if (y < int(s_Band.bar_y0) || y >= int(s_Band.bar_y1))	return 0;
	if (x < int(s_Band.bar_x0) || x >= int(s_Band.bar_x1))	return 0;
	// menus and buttons take the mouse; the empty middle drags the window
	if (x < int(s_Band.menus_end_x))				return HTCLIENT;
	if (x >= int(s_Band.buttons_x0))				return HTCLIENT;
	return HTCAPTION;
}
} // namespace

namespace UITitleBar
{
void Install()
{
	EditorTitleBar::SetHitTest(&HitTest);
}

void DrawLeft()
{
	const float h = ImGui::GetFrameHeight();
	const ImVec2 wp = ImGui::GetWindowPos();
	const ImVec2 ws = ImGui::GetWindowSize();
	s_Band.bar_x0 = wp.x;
	s_Band.bar_y0 = wp.y;
	s_Band.bar_x1 = wp.x + ws.x;
	s_Band.bar_y1 = wp.y + ws.y;

	// lazy: Install() runs before the renderer exists, a texture cannot
	static bool icon_tried = false;
	if (!icon_tried) { icon_tried = true; LoadIconTexture(); }
	if (s_Icon)
	{
		const float pad = 3.f;
		const float side = h - 2.f * pad;
		ImDrawList* dl = ImGui::GetWindowDrawList();
		const ImVec2 p = ImGui::GetCursorScreenPos();
		// the icon sits on a soft accent glow so it reads as the app's mark,
		// not as one more toolbar button
		dl->AddCircleFilled(ImVec2(p.x + pad + side * 0.5f, p.y + h * 0.5f), side * 0.78f, ThemeColor(XFinedTheme::ColorToken::Accent, 0.16f));
		dl->AddImage(s_Icon, ImVec2(p.x + pad, p.y + pad), ImVec2(p.x + pad + side, p.y + pad + side));
		ImGui::Dummy(ImVec2(side + 2.f * pad + 2.f, h));
	}
}

void DrawRight()
{
	const float h = ImGui::GetFrameHeight();
	const float side = h;
	const float btn_w = side * 1.55f;
	const ImVec2 wp = ImGui::GetWindowPos();
	const ImVec2 ws = ImGui::GetWindowSize();

	s_Band.menus_end_x = ImGui::GetCursorScreenPos().x;
	s_Band.buttons_x0 = wp.x + ws.x - 3.f * btn_w;

	// the title lives in the drag band, dimmed, centered on the free space
	{
		xr_string title = "XFined Editor";
		if (EditorProject::Active())
		{
			title = EditorProject::Name();
			title += " - XFined Editor";
		}
		// centered on the WINDOW, like every OS caption - the free band is not
		// symmetric (menus left, buttons right), centering on it drifts the text
		const ImVec2 sz = ImGui::CalcTextSize(title.c_str());
		const float lo = s_Band.menus_end_x + 12.f;
		const float hi = s_Band.buttons_x0 - 8.f - sz.x;
		if (hi >= lo)
		{
			float tx = (s_Band.bar_x0 + s_Band.bar_x1 - sz.x) * 0.5f;
			tx = std::min(std::max(tx, lo), hi);
			ImGui::GetWindowDrawList()->AddText(ImVec2(tx, wp.y + (h - sz.y) * 0.5f),
				ImGui::GetColorU32(ImGuiCol_TextDisabled), title.c_str());
		}
	}

	// frameless right-aligned button strip, flush with the window edge
	ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(0.f, 0.f));
	ImGui::SetCursorScreenPos(ImVec2(s_Band.buttons_x0, wp.y));
	if (WindowButton(btnMin, side))
		::ShowWindow(EDevice->m_hWnd, SW_MINIMIZE);
	ImGui::SameLine(0.f, 0.f);
	if (WindowButton(btnMax, side))
		::ShowWindow(EDevice->m_hWnd, ::IsZoomed(EDevice->m_hWnd) ? SW_RESTORE : SW_MAXIMIZE);
	ImGui::SameLine(0.f, 0.f);
	if (WindowButton(btnClose, side))
		::PostMessageW(EDevice->m_hWnd, WM_CLOSE, 0, 0);	// the COMMAND_EXIT path
	ImGui::PopStyleVar();

	s_Band.valid = true;
}

void McpInfo(xr_string& out)
{
	char tmp[512];
	sprintf_s(tmp,
		"{\"ok\":true,\"enabled\":%s,\"band\":[%d,%d,%d,%d],\"menus_end\":%d,\"buttons_x\":%d,"
		"\"maximized\":%s,\"icon\":%s}",
		EditorTitleBar::Enabled() ? "true" : "false",
		int(s_Band.bar_x0), int(s_Band.bar_y0), int(s_Band.bar_x1), int(s_Band.bar_y1),
		int(s_Band.menus_end_x), int(s_Band.buttons_x0),
		::IsZoomed(EDevice->m_hWnd) ? "true" : "false",
		s_Icon ? "true" : "false");
	out = tmp;
}
} // namespace UITitleBar
