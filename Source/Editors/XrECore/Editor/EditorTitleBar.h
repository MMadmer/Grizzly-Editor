#pragma once

// Custom window chrome. An editor that wants its ImGui main bar to BE the
// caption registers a hit-test provider; the window then loses the system
// title bar (WM_NCCALCSIZE) while keeping the resize borders, Aero snap,
// the DWM shadow and the minimize/maximize animations. Editors that never
// register (ActorEditor and the rest) keep the classic caption untouched.
//
// The provider answers in CLIENT pixels with an HT* code:
//   HTCAPTION - empty band: drag, double-click maximizes, right-click menu
//   HTCLIENT  - interactive spot of the band (menu label, window button)
//   0         - not in the band at all
namespace EditorTitleBar
{
typedef int (*THitTest)(int client_x, int client_y);

ECORE_API void	SetHitTest(THitTest fn);
ECORE_API bool	Enabled();
ECORE_API int	HitTest(int client_x, int client_y);

// Height of the frame part Windows keeps above the client when a captionless
// window is maximized - the band the caption trick has to give back.
ECORE_API int	MaximizedTopPad(void* hwnd);
}
