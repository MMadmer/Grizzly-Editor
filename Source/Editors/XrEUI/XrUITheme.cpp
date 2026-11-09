#include "stdafx.h"
#include "imgui_internal.h"
#include "XrUIKitHooks.h"

namespace
{
using GrizzlyTheme::ColorToken;

bool g_MultiViewport = false;
GrizzlyTheme::Preset g_Current = GrizzlyTheme::Preset::GrizzlyPurple;

// rgb plus an 8-bit alpha in the top byte; 0 there means opaque, so the table stays readable
using Palette = u32[u32(ColorToken::Count)];

// docs/UI_DESIGN.md section 2. The bold values of the document are the ones the editor had.
const Palette kPurple =
{
	0x202025, 0x1c1b20, 0x2a2931, 0x3d3947,
	0x7971bd, 0x554f72, 0xa79fed, 0x464150,
	0xf2effb, 0x8e8a99, 0xe06c75, 0xe5c07b, 0x98c379,

	0x17161b, 0x151419, 0x26252c, 0x28272e,
	0x302e38, 0x2e2c35, 0x36343f, 0x34323d, 0x3d3a47,
	0x2c2a33, 0x524d5f,
	0xbdb8c9, 0x5f5b69,
	0x6a62ae, 0x7971bd, 0x5c5599,
	0x8a3b43, 0xa3464f, 0x74323a,
	0xd19a66,
	0xe06c75, 0x98c379, 0x61afef,
	0x990c0b0e,
};

// Graphite changes the accent family only
Palette kGraphite;

const u32* PaletteFor(GrizzlyTheme::Preset preset)
{
	if (preset != GrizzlyTheme::Preset::Graphite)
		return kPurple;
	if (!kGraphite[0])
	{
		memcpy(kGraphite, kPurple, sizeof(kGraphite));
		kGraphite[u32(ColorToken::Accent)] = 0x727782;
		kGraphite[u32(ColorToken::AccentActive)] = 0x9399a5;
		kGraphite[u32(ColorToken::Selection)] = 0x454951;
		kGraphite[u32(ColorToken::AccentHovered)] = 0x50545d;
		kGraphite[u32(ColorToken::Primary)] = 0x60656f;
		kGraphite[u32(ColorToken::PrimaryHovered)] = 0x727782;
		kGraphite[u32(ColorToken::PrimaryPressed)] = 0x53575f;
	}
	return kGraphite;
}

ImVec4 Col(const u32* palette, ColorToken token, float alpha = 1.f)
{
	const u32 v = palette[u32(token)];
	const float own = (v >> 24) ? float(v >> 24) / 255.f : 1.f;
	return ImVec4(float((v >> 16) & 0xff) / 255.f, float((v >> 8) & 0xff) / 255.f, float(v & 0xff) / 255.f, own * alpha);
}

// Every size in pt; ScaleAllSizes turns them into px once, in one place (XUI::SetDpi).
void ApplyMetrics(ImGuiStyle& style)
{
	style.Alpha = 1.f;
	style.DisabledAlpha = .4f;
	style.WindowPadding = ImVec2(8.f, 8.f);
	style.WindowRounding = 6.f;
	style.WindowBorderSize = 1.f;
	style.WindowMinSize = ImVec2(32.f, 32.f);
	style.WindowTitleAlign = ImVec2(0.f, .5f);
	style.WindowMenuButtonPosition = ImGuiDir_None;
	style.ChildRounding = 4.f;
	style.ChildBorderSize = 1.f;
	style.PopupRounding = 6.f;
	style.PopupBorderSize = 1.f;
	style.FramePadding = ImVec2(8.f, 4.f);
	style.FrameRounding = 4.f;
	style.FrameBorderSize = 1.f;
	style.ItemSpacing = ImVec2(8.f, 4.f);
	style.ItemInnerSpacing = ImVec2(6.f, 4.f);
	style.CellPadding = ImVec2(6.f, 3.f);
	style.TouchExtraPadding = ImVec2(0.f, 0.f);
	style.IndentSpacing = 14.f;
	style.ColumnsMinSpacing = 6.f;
	style.ScrollbarSize = 10.f;
	style.ScrollbarRounding = 5.f;
	style.ScrollbarPadding = 1.f;
	style.GrabMinSize = 12.f;
	style.GrabRounding = 4.f;
	style.LogSliderDeadzone = 4.f;
	style.ImageRounding = 4.f;
	style.ImageBorderSize = 0.f;
	style.TabRounding = 4.f;
	style.TabBorderSize = 0.f;
	// close crosses on hover only, selected or not
	style.TabCloseButtonMinWidthSelected = 0.f;
	style.TabCloseButtonMinWidthUnselected = 0.f;
	style.TabBarBorderSize = 1.f;
	style.TabBarOverlineSize = 2.f;
	style.TreeLinesSize = 1.f;
	style.TreeLinesRounding = 2.f;
	style.MenuItemRounding = 4.f;
	style.SelectableRounding = 4.f;
	style.DragDropTargetRounding = 4.f;
	style.DragDropTargetBorderSize = 2.f;
	style.DragDropTargetPadding = 3.f;
	style.ColorButtonPosition = ImGuiDir_Right;
	style.ButtonTextAlign = ImVec2(.5f, .5f);
	style.SelectableTextAlign = ImVec2(0.f, .5f);
	style.InputTextCursorSize = 1.f;
	style.SeparatorSize = 1.f;
	style.SeparatorTextBorderSize = 1.f;
	style.SeparatorTextPadding = ImVec2(12.f, 3.f);
	style.DisplayWindowPadding = ImVec2(19.f, 19.f);
	style.DisplaySafeAreaPadding = ImVec2(3.f, 3.f);
	style.DockingSeparatorSize = 2.f;
	style.MouseCursorScale = 1.f;
	style.AntiAliasedLines = true;
	style.AntiAliasedLinesUseTex = true;
	style.AntiAliasedFill = true;
	style.CurveTessellationTol = 1.25f;
	style.CircleTessellationMaxError = .3f;

	// tooltips after 0.45 s of a resting mouse, disabled items included (their reason is the tip)
	style.HoverStationaryDelay = .15f;
	style.HoverDelayShort = .15f;
	style.HoverDelayNormal = .45f;
	style.HoverFlagsForTooltipMouse = ImGuiHoveredFlags_Stationary | ImGuiHoveredFlags_DelayNormal | ImGuiHoveredFlags_AllowWhenDisabled;
	style.HoverFlagsForTooltipNav = ImGuiHoveredFlags_NoSharedDelay | ImGuiHoveredFlags_DelayNormal | ImGuiHoveredFlags_AllowWhenDisabled;
}

void ApplyPalette(ImGuiStyle& style, const u32* p)
{
	ImVec4* c = style.Colors;
	c[ImGuiCol_Text] = Col(p, ColorToken::Text);
	c[ImGuiCol_TextDisabled] = Col(p, ColorToken::Muted);
	c[ImGuiCol_WindowBg] = Col(p, ColorToken::Base);
	c[ImGuiCol_ChildBg] = Col(p, ColorToken::Inset);
	c[ImGuiCol_PopupBg] = Col(p, ColorToken::Overlay);
	c[ImGuiCol_Border] = Col(p, ColorToken::Border);
	c[ImGuiCol_BorderShadow] = ImVec4(0.f, 0.f, 0.f, 0.f);
	c[ImGuiCol_FrameBg] = Col(p, ColorToken::Input);
	c[ImGuiCol_FrameBgHovered] = Col(p, ColorToken::InputHovered);
	c[ImGuiCol_FrameBgActive] = Col(p, ColorToken::Pressed);
	c[ImGuiCol_TitleBg] = Col(p, ColorToken::Chrome);
	c[ImGuiCol_TitleBgActive] = Col(p, ColorToken::Raised);
	c[ImGuiCol_TitleBgCollapsed] = Col(p, ColorToken::Chrome);
	c[ImGuiCol_MenuBarBg] = Col(p, ColorToken::Chrome);
	c[ImGuiCol_ScrollbarBg] = ImVec4(0.f, 0.f, 0.f, 0.f);
	c[ImGuiCol_ScrollbarGrab] = Col(p, ColorToken::Border);
	c[ImGuiCol_ScrollbarGrabHovered] = Col(p, ColorToken::BorderStrong);
	c[ImGuiCol_ScrollbarGrabActive] = Col(p, ColorToken::Muted);
	c[ImGuiCol_CheckMark] = Col(p, ColorToken::AccentBright);
	c[ImGuiCol_CheckboxSelectedBg] = Col(p, ColorToken::Accent, .4f);
	c[ImGuiCol_SliderGrab] = Col(p, ColorToken::Accent);
	c[ImGuiCol_SliderGrabActive] = Col(p, ColorToken::AccentBright);
	c[ImGuiCol_Button] = Col(p, ColorToken::Button);
	c[ImGuiCol_ButtonHovered] = Col(p, ColorToken::ButtonHovered);
	c[ImGuiCol_ButtonActive] = Col(p, ColorToken::Pressed);
	c[ImGuiCol_Header] = Col(p, ColorToken::Selection);
	c[ImGuiCol_HeaderHovered] = Col(p, ColorToken::ButtonHovered);
	c[ImGuiCol_HeaderActive] = Col(p, ColorToken::SelectionFocused);
	c[ImGuiCol_Separator] = Col(p, ColorToken::Border);
	c[ImGuiCol_SeparatorHovered] = Col(p, ColorToken::AccentBright, .6f);
	c[ImGuiCol_SeparatorActive] = Col(p, ColorToken::Accent);
	c[ImGuiCol_ResizeGrip] = ImVec4(0.f, 0.f, 0.f, 0.f);
	c[ImGuiCol_ResizeGripHovered] = Col(p, ColorToken::Accent, .5f);
	c[ImGuiCol_ResizeGripActive] = Col(p, ColorToken::Accent);
	c[ImGuiCol_InputTextCursor] = Col(p, ColorToken::AccentBright);
	c[ImGuiCol_TabHovered] = Col(p, ColorToken::Hover);
	c[ImGuiCol_Tab] = Col(p, ColorToken::Chrome);
	c[ImGuiCol_TabSelected] = Col(p, ColorToken::Base);
	c[ImGuiCol_TabSelectedOverline] = Col(p, ColorToken::AccentBright);
	c[ImGuiCol_TabDimmed] = Col(p, ColorToken::Chrome);
	c[ImGuiCol_TabDimmedSelected] = Col(p, ColorToken::Base);
	// only the focused panel's tab carries the overline
	c[ImGuiCol_TabDimmedSelectedOverline] = ImVec4(0.f, 0.f, 0.f, 0.f);
	c[ImGuiCol_DockingPreview] = Col(p, ColorToken::Accent, .6f);
	c[ImGuiCol_DockingEmptyBg] = Col(p, ColorToken::Canvas);
	c[ImGuiCol_PlotLines] = Col(p, ColorToken::TextSecondary);
	c[ImGuiCol_PlotLinesHovered] = Col(p, ColorToken::Warning);
	c[ImGuiCol_PlotHistogram] = Col(p, ColorToken::AccentBright);
	c[ImGuiCol_PlotHistogramHovered] = Col(p, ColorToken::Text);
	c[ImGuiCol_TableHeaderBg] = Col(p, ColorToken::Raised);
	c[ImGuiCol_TableBorderStrong] = Col(p, ColorToken::Border);
	c[ImGuiCol_TableBorderLight] = Col(p, ColorToken::BorderSubtle);
	c[ImGuiCol_TableRowBg] = ImVec4(0.f, 0.f, 0.f, 0.f);
	c[ImGuiCol_TableRowBgAlt] = ImVec4(1.f, 1.f, 1.f, .02f);
	c[ImGuiCol_TextLink] = Col(p, ColorToken::AccentBright);
	c[ImGuiCol_TextSelectedBg] = Col(p, ColorToken::SelectionFocused);
	c[ImGuiCol_TreeLines] = Col(p, ColorToken::BorderSubtle);
	c[ImGuiCol_DragDropTarget] = Col(p, ColorToken::AccentBright);
	c[ImGuiCol_DragDropTargetBg] = Col(p, ColorToken::Accent, .16f);
	c[ImGuiCol_UnsavedMarker] = Col(p, ColorToken::Warning);
	c[ImGuiCol_NavCursor] = Col(p, ColorToken::AccentBright);
	c[ImGuiCol_NavWindowingHighlight] = Col(p, ColorToken::Text, .7f);
	c[ImGuiCol_NavWindowingDimBg] = Col(p, ColorToken::Dim, .75f);
	c[ImGuiCol_ModalWindowDimBg] = Col(p, ColorToken::Dim);
}

// ImGui's arrows as the kit's chevrons (GImGuiRenderArrow): the same glyph the toolbar and the sections use, on a
// whole-pixel square so its stroke stays crisp
void KitArrow(ImDrawList* dl, ImVec2 center, float side, ImU32 col, ImGuiDir dir)
{
	const XUI::Icon icon = dir == ImGuiDir_Up ? XUI::Icon::ChevronUp : dir == ImGuiDir_Down ? XUI::Icon::ChevronDown
		: dir == ImGuiDir_Left ? XUI::Icon::ChevronLeft : XUI::Icon::ChevronRight;
	const float px = IM_ROUND(side);
	const ImVec2 min(IM_ROUND(center.x - px * .5f), IM_ROUND(center.y - px * .5f));
	XUI::DrawIconRect(dl, icon, min, ImVec2(min.x + px, min.y + px), col);
}

void Build()
{
	ImGuiStyle style;
	ApplyMetrics(style);
	ApplyPalette(style, PaletteFor(g_Current));
	// a platform window cannot be rounded, and a translucent one shows the desktop through
	if (g_MultiViewport)
	{
		style.WindowRounding = 0.f;
		style.Colors[ImGuiCol_WindowBg].w = 1.f;
	}
	const float dpi = XUI::Dpi();
	if (dpi != 1.f)
		style.ScaleAllSizes(dpi);
	// the size of a font is its line box: Body 16, and the monitor's scale on top of it
	style.FontSizeBase = XUI::FontBaseSize;
	style.FontScaleMain = 1.f;
	style.FontScaleDpi = dpi;
	// the panel tab strip is 26 pt: the dockspace pads its tab bars by what a frame is short of that
	GImGuiDockTabBarExtra = ImMax(0.f, floorf(26.f * dpi - (XUI::FontBaseSize * dpi + style.FramePadding.y * 2.f) + .5f));
	GImGuiRenderArrow = &KitArrow;
	// a number starts at the left of its field, like a text
	GImGuiDragTextAlignX = 0.f;
	ImGui::GetStyle() = style;
}
}

void XUI::Hooks::RebuildStyle()
{
	if (ImGui::GetCurrentContext())
		Build();
}

void GrizzlyTheme::Initialize(bool multi_viewport)
{
	g_MultiViewport = multi_viewport;
	Apply(g_Current);
}

void GrizzlyTheme::Apply(Preset preset)
{
	if (!IsValid(u32(preset)))
		preset = Default();
	g_Current = preset;
	ImGuiContext* ctx = ImGui::GetCurrentContext();
	if (!ctx)
		return;
	// a style swapped under a running frame would unbalance its pushed vars: wait for the next one
	if (ctx->WithinFrameScope)
		XUI::Hooks::RequestStyle();
	else
		Build();
}

GrizzlyTheme::Preset GrizzlyTheme::Current()
{
	return g_Current;
}

GrizzlyTheme::Preset GrizzlyTheme::Default()
{
	return Preset::GrizzlyPurple;
}

bool GrizzlyTheme::IsValid(u32 preset)
{
	return preset < u32(Preset::Count);
}

bool GrizzlyTheme::TryParse(LPCSTR value, Preset& preset)
{
	if (!value || !value[0])
		return false;
	for (u32 i = 0; i < u32(Preset::Count); ++i)
	{
		const Preset candidate = Preset(i);
		if (0 == _stricmp(value, Key(candidate)) || 0 == _stricmp(value, Name(candidate)))
		{
			preset = candidate;
			return true;
		}
	}
	return false;
}

LPCSTR GrizzlyTheme::Key(Preset preset)
{
	switch (preset)
	{
	case Preset::GrizzlyPurple:	return "grizzly-purple";
	case Preset::Graphite:		return "graphite";
	default:					return "grizzly-purple";
	}
}

LPCSTR GrizzlyTheme::Name(Preset preset)
{
	switch (preset)
	{
	case Preset::GrizzlyPurple:	return "Grizzly Purple";
	case Preset::Graphite:		return "Graphite";
	default:					return "Grizzly Purple";
	}
}

u32 GrizzlyTheme::Rgb(ColorToken token)
{
	return token < ColorToken::Count ? PaletteFor(g_Current)[u32(token)] & 0xffffff : PaletteFor(g_Current)[u32(ColorToken::Text)] & 0xffffff;
}

ImVec4 GrizzlyTheme::Vec4(ColorToken token, float alpha)
{
	return Col(PaletteFor(g_Current), token < ColorToken::Count ? token : ColorToken::Text, alpha);
}

ImU32 GrizzlyTheme::U32(ColorToken token, float alpha)
{
	return ImGui::ColorConvertFloat4ToU32(Vec4(token, alpha));
}
