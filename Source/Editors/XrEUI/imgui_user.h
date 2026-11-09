#pragma once
namespace ImGui
{
	IMGUI_API float GetWindowBarHeight();
	IMGUI_API ImVec2        CalcItemSize(ImVec2 size, float default_w, float default_h);
	IMGUI_API bool OpenPopupOnItemClick2(const char* str_id, ImGuiPopupFlags popup_flags);
	IMGUI_API bool ImageButton(const char* str_id, ImTextureRef tex_ref, const ImVec2& image_size,
		const ImVec2& uv0, const ImVec2& uv1, int frame_padding,
		const ImVec4& bg_col = ImVec4(0, 0, 0, 0), const ImVec4& tint_col = ImVec4(1, 1, 1, 1));
	IMGUI_API void PushMixedValue(bool mixed);

    IMGUI_API bool InputFloat(const char* label, float* v, float step, float step_fast, int dec, ImGuiInputTextFlags flags = 0);
    IMGUI_API bool InputFloat2(const char* label, float v[2], int dec, ImGuiInputTextFlags flags = 0);
    IMGUI_API bool InputFloat3(const char* label, float v[3], int dec, ImGuiInputTextFlags flags = 0);
    IMGUI_API bool InputFloat4(const char* label, float v[4], int dec, ImGuiInputTextFlags flags = 0);
    IMGUI_API bool BeginPopupModal(const char* name, bool* p_open, ImGuiWindowFlags flags, bool open_always);
    // The modal of that name, drawn through the overload above, closes the next time it is drawn the way its
    // caption's cross takes it (MCP ui_dialog close). A request nobody takes within a few frames lapses.
    IMGUI_API void RequestPopupModalClose(const char* name);
    IMGUI_API bool ArrowButton(const char* str_id, ImGuiDir dir, ImVec2 size, ImGuiButtonFlags flags);
}
// Grizzly Editor: px a dock node's tab bar is taller than a frame (the panel tab strip is 26 pt). The dockspace submits its
// tab bars with that much more FramePadding.y; a docked window's title bar, which is that tab bar, takes it in imgui.cpp.
extern IMGUI_API float GImGuiDockTabBarExtra;
// Grizzly Editor: set by the theme (XrUITheme.cpp). ImGui's arrows - the combo button, a tree node, a submenu, a
// sorted column - are drawn by it in their box (centre and side in px) instead of as a solid triangle; null keeps
// ImGui's triangle.
typedef void (*ImGuiRenderArrowFn)(ImDrawList* draw_list, ImVec2 center, float side, ImU32 col, ImGuiDir dir);
extern IMGUI_API ImGuiRenderArrowFn GImGuiRenderArrow;
// Grizzly Editor: set by the theme (XrUITheme.cpp). Where a drag's value sits in its frame, 0 left .. 1 right, inside the
// frame padding; ImGui centres it (0.5).
extern IMGUI_API float GImGuiDragTextAlignX;
