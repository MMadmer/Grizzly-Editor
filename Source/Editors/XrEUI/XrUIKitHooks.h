#pragma once

// Calls between the kit and the rest of XrEUI (XrUIManager, the theme). Not part of the kit's API:
// nothing outside XrEUI includes this.
namespace XUI
{
namespace Hooks
{
// after ImGui::CreateContext: the .ini handler, the startup scale
void ContextCreated(HWND hwnd);
// before ImGui::NewFrame: a style or scale asked for mid-frame, the fonts if nobody loaded them
void BeforeNewFrame();
// the next DrawOverlays belongs to a progress frame: toasts only, the dialogs wait
void SetProgressFrame(bool progress);
// the theme asks for a rebuild at the next frame / does it now
void RequestStyle();
void RebuildStyle();
// files that landed where nothing listens are forgotten (XrUIFileDrop.cpp)
void FileDropFrameEnd();
// the fonts of the kit (XrUIFonts.cpp)
bool FontsLoaded();
}
}
