#pragma once

namespace GrizzlyTheme
{
enum class Preset : u32
{
	GrizzlyPurple = 0,
	Graphite,
	Count,
};

// The colour roles of docs/UI_DESIGN.md section 2. The first 13 keep their order and meaning (and,
// but for Input, their values); the rest are appended. The aliases name the same role the way the
// design document does.
enum class ColorToken : u32
{
	Background = 0,		// base: panels, documents, toolbar, active tab
	Panel,				// inset: lists, tables, grids, tiles, log
	Input,
	Border,
	Accent,
	AccentHovered,		// selection.focused
	AccentActive,		// accent.bright: focus, check mark, caret, links, overlines
	Selection,
	Text,
	Muted,
	Error,
	Warning,
	Success,

	Chrome,				// caption, status bar, inactive tabs, dock strips
	Canvas,				// viewport, graph, previews
	Raised,				// headers, cards, node bodies
	Overlay,			// menus, popups, tooltips, dialogs, toasts
	InputHovered,
	Hover,				// hover fill of rows and ghost buttons
	Pressed,
	Button,
	ButtonHovered,
	BorderSubtle,
	BorderStrong,
	TextSecondary,
	TextDisabled,
	Primary,
	PrimaryHovered,
	PrimaryPressed,
	Danger,
	DangerHovered,
	DangerPressed,
	Required,
	AxisX,
	AxisY,
	AxisZ,
	Dim,				// the modal dim, used at 60%
	Count,

	Base = Background,
	Inset = Panel,
	AccentBright = AccentActive,
	SelectionFocused = AccentHovered,
	TextMuted = Muted,
};

// Builds the style from the pt metrics at XUI::Dpi(). Called once the ImGui context exists.
XREUI_API void Initialize(bool multi_viewport);
// Re-applies the style; a call made inside a frame takes effect before the next one.
XREUI_API void Apply(Preset preset);
XREUI_API Preset Current();
XREUI_API Preset Default();
XREUI_API bool IsValid(u32 preset);
XREUI_API bool TryParse(LPCSTR value, Preset& preset);
XREUI_API LPCSTR Key(Preset preset);
XREUI_API LPCSTR Name(Preset preset);
XREUI_API u32 Rgb(ColorToken token);
// The token of the current preset as ImGui wants it; alpha multiplies the token's own.
XREUI_API ImU32 U32(ColorToken token, float alpha = 1.f);
XREUI_API ImVec4 Vec4(ColorToken token, float alpha = 1.f);
}
