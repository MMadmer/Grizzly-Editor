#include "stdafx.h"
#if defined(USE_DX11)
#include "imgui_impl_dx11.h"
#else
#include "imgui_impl_dx9.h"
#endif
#include "imgui_impl_win32.h"
#include "imgui_internal.h"
#include "XrUIKitHooks.h"
#include "XrUIShell.h"

// A minimized editor draws only when a screenshot asks for a frame, and that frame has to
// show the editor the way it looks restored. ImGui draws nothing of a minimized viewport and
// merges no window into it, so for such a frame it is told the main window is restored: where
// it stood last, at the size the editor draws at. Not while a window of the editor is in front
// (one dragged out of it): somebody is looking then, and a panel dropped where the main window
// would be must not vanish into it.
namespace
{
	bool (*s_GetWindowMinimized)(ImGuiViewport*) = nullptr;
	ImVec2 (*s_GetWindowPos)(ImGuiViewport*) = nullptr;
	bool s_DrawRestored = false;
	ImVec2 s_RestoredPos;

	bool GetWindowMinimized(ImGuiViewport* viewport)
	{
		return !(s_DrawRestored && viewport == ImGui::GetMainViewport()) && s_GetWindowMinimized(viewport);
	}

	ImVec2 GetWindowPos(ImGuiViewport* viewport)
	{
		return s_DrawRestored && viewport == ImGui::GetMainViewport() ? s_RestoredPos : s_GetWindowPos(viewport);
	}

	// the minimized main window itself may stay the foreground one, with nothing to show
	bool OtherEditorWindowInFront(HWND main)
	{
		DWORD process = 0;
		HWND front = ::GetForegroundWindow();
		if (front && front != main)
			::GetWindowThreadProcessId(front, &process);
		return process == ::GetCurrentProcessId();
	}
}

XrUIManager::XrUIManager()
{
}

XrUIManager::~XrUIManager()
{
}

#if defined(USE_DX11)
void XrUIManager::Initialize(HWND hWnd, ID3D11Device* device, ID3D11DeviceContext* context, const char* ini_path)
#else
void XrUIManager::Initialize(HWND hWnd, IDirect3DDevice9* device, const char* ini_path)
#endif
{
    IMGUI_CHECKVERSION();
    ImGui::CreateContext();
    ImGuiIO& io = ImGui::GetIO(); (void)io;
    xr_strcpy(m_name_ini, ini_path);
    io.IniFilename = m_name_ini;
    //io.ConfigFlags |= ImGuiConfigFlags_NavEnableKeyboard;     // Enable Keyboard Controls
    //io.ConfigFlags |= ImGuiConfigFlags_NavEnableGamepad;      // Enable Gamepad Controls
    io.ConfigFlags |= ImGuiConfigFlags_NavEnableKeyboard;       // Enable Keyboard Controls
  //io.ConfigFlags |= ImGuiConfigFlags_NavEnableGamepad;      // Enable Gamepad Controls
    io.ConfigFlags |= ImGuiConfigFlags_DockingEnable;           // Enable Docking
    // Platform windows only when the author asked for panels outside the editor (a preference that takes a
    // restart): a panel in a window of its own is in no screenshot of the editor, and every modal and panel
    // is clamped inside the editor's own window otherwise.
    if (XUI::Shell::FloatingPanels())
        io.ConfigFlags |= ImGuiConfigFlags_ViewportsEnable;
    // a layout written by an older editor names windows and dock nodes that no longer exist: it is not read
    // once, and the next save replaces it
    if (XUI::Shell::SkipLayoutOnce())
        ImGui::GetCurrentContext()->SettingsLoaded = true;
    // the monitor's scale, or -uiscale, goes through XUI::SetDpi like every later change; the
    // fonts load as a startup stage of their own (XUI::LoadFonts), or with the first frame
    XUI::Hooks::ContextCreated(hWnd);
    GrizzlyTheme::Initialize((io.ConfigFlags & ImGuiConfigFlags_ViewportsEnable) != 0);
    ImGui_ImplWin32_Init(hWnd);
    XUI::FileDropInstall(hWnd);
    #if defined(USE_DX11)
    ImGui_ImplDX11_Init(device, context);
#else
    ImGui_ImplDX9_Init(device);
#endif

    m_hWnd = hWnd;
    ImGuiPlatformIO& platform = ImGui::GetPlatformIO();
    if (platform.Platform_GetWindowMinimized && platform.Platform_GetWindowPos)
    {
        s_GetWindowMinimized = platform.Platform_GetWindowMinimized;
        s_GetWindowPos = platform.Platform_GetWindowPos;
        platform.Platform_GetWindowMinimized = GetWindowMinimized;
        platform.Platform_GetWindowPos = GetWindowPos;
    }
    // an editor started minimized never had a client area: its restored frame will do
    WINDOWPLACEMENT placement = { sizeof(placement) };
    if (::GetWindowPlacement(hWnd, &placement))
        s_RestoredPos = ImVec2(float(placement.rcNormalPosition.left), float(placement.rcNormalPosition.top));
}

void XrUIManager::NewPlatformFrame()
{
    ImGui_ImplWin32_NewFrame();
    s_DrawRestored = false;
    if (!m_hWnd || !s_GetWindowMinimized)
        return;
    if (!::IsIconic(m_hWnd))
    {
        POINT origin = { 0, 0 };
        ::ClientToScreen(m_hWnd, &origin);
        s_RestoredPos = ImVec2(float(origin.x), float(origin.y));
    }
    else if (m_RenderWidth > 0 && m_RenderHeight > 0 && !OtherEditorWindowInFront(m_hWnd))
    {
        s_DrawRestored = true;
        ImGui::GetIO().DisplaySize = ImVec2(float(m_RenderWidth), float(m_RenderHeight));
    }
}

bool XrUIManager::TopModal(xr_string& title, float rect[4], xr_string* id) const
{
    ImGuiWindow* modal = ImGui::GetTopMostAndVisiblePopupModal();
    const ImGuiViewport* main = ImGui::GetMainViewport();
    if (!modal || !modal->Viewport || modal->Viewport->ID != main->ID)
        return false;
    const char* name = modal->Name;
    title.assign(name, ImGui::FindRenderedTextEnd(name));
    if (id)
    {
        const char* hash = strstr(name, "###");
        *id = hash ? xr_string(hash + 3) : title;
    }
    // no platform on Windows sets the viewport's own scale: the draw data falls back the same way
    const ImVec2 scale = main->FramebufferScale.x != 0.0f ? main->FramebufferScale : ImGui::GetIO().DisplayFramebufferScale;
    rect[0] = (modal->Pos.x - main->Pos.x) * scale.x;
    rect[1] = (modal->Pos.y - main->Pos.y) * scale.y;
    rect[2] = modal->Size.x * scale.x;
    rect[3] = modal->Size.y * scale.y;
    return true;
}

void XrUIManager::Destroy()
{
    XUI::FileDropRemove();
    #if defined(USE_DX11)
    ImGui_ImplDX11_Shutdown();
#else
    ImGui_ImplDX9_Shutdown();
#endif
    ImGui_ImplWin32_Shutdown();
    ImGui::DestroyContext();
}

void XrUIManager::ResetBegin()
{
    #if defined(USE_DX11)
    ImGui_ImplDX11_InvalidateDeviceObjects();
#else
    ImGui_ImplDX9_InvalidateDeviceObjects();
#endif

}

void XrUIManager::ResetEnd()
{
    #if defined(USE_DX11)
    ImGui_ImplDX11_CreateDeviceObjects();
#else
    ImGui_ImplDX9_CreateDeviceObjects();
#endif
}


void XrUIManager::OnDrawUI()
{
}

void XrUIManager::ApplyShortCut(DWORD Key)
{
    if ((ImGui::GetIO().WantTextInput))return;
    // first taker wins: the shell's own chords, then the active document; the scene's bindings act only on the
    // Level tab with nothing modal up and no panel that answers its own keys focused
    const ImGuiIO& io = ImGui::GetIO();
    if (XUI::Shell::ShellChord(Key, io.KeyCtrl, io.KeyShift, io.KeyAlt))
    {
        static bool s_Told = false;
        if (!s_Told)
        {
            s_Told = true;
            Msg("* shortcuts: Ctrl+S, Ctrl+Tab, Ctrl+F4, Ctrl+Space, Ctrl+`, Ctrl+Shift+P and F11 belong to the editor's frame");
        }
        return;
    }
    if (!XUI::Shell::SceneKeysAllowed())
        return;
	bool IsFail = true;
	if (Key >= 'A' && Key <= 'Z')
	{
		IsFail = false;
	}
	else if (Key >= '0' && Key <= '9')
	{
		IsFail = false;
	}
    else
    {
        switch (Key)
        {
        case VK_LEFT:
        case VK_RIGHT:
        case VK_UP:
        case VK_DOWN:
        case VK_NUMPAD0:
        case VK_NUMPAD1:
        case VK_NUMPAD2:
        case VK_NUMPAD3:
        case VK_NUMPAD4:
        case VK_NUMPAD5:
        case VK_NUMPAD6:
        case VK_NUMPAD7:
        case VK_NUMPAD8:
        case VK_NUMPAD9:
        case VK_F1:
        case VK_F2:
        case VK_F3:
        case VK_F4:
        case VK_F5:
        case VK_F6:
        case VK_F7:
        case VK_F8:
        case VK_F9:
        case VK_F10:
        case VK_F11:
        case VK_F12:
        case VK_DELETE:
        case VK_ADD:
        case VK_SUBTRACT:
        case VK_MULTIPLY:
        case VK_DIVIDE:
        case VK_OEM_PLUS:
        case VK_OEM_MINUS:
        case VK_OEM_1:
        case VK_OEM_COMMA:
        case VK_OEM_PERIOD:
        case VK_OEM_2:
        case VK_OEM_4:
        case VK_OEM_5:
        case VK_OEM_6:
        case VK_OEM_7:
        case VK_SPACE:
        case VK_CANCEL:
        case VK_RETURN:
        case VK_ESCAPE:
        // navigation block: End drops the selection to the ground, the rest
        // are free for user bindings
        case VK_END:
        case VK_HOME:
        case VK_PRIOR:
        case VK_NEXT:
        case VK_INSERT:
            IsFail = false;
            break;
        default:
            break;
        }
    }
    if (IsFail)return;

    int ShiftState = ssNone;

    if (ImGui::GetIO().KeyShift)ShiftState |= ssShift;
    if (ImGui::GetIO().KeyCtrl)ShiftState |= ssCtrl;
    if (ImGui::GetIO().KeyAlt)ShiftState |= ssAlt;


    if (ImGui::IsMouseDown(ImGuiMouseButton_Left))ShiftState |= ssLeft;
    if (ImGui::IsMouseDown(ImGuiMouseButton_Right))ShiftState |= ssRight;
    ApplyShortCut(Key, ShiftState);
}



void XrUIManager::Push(XrUI* ui, bool need_deleted)
{
	m_UIArray.push_back(ui);
	ui->Flags.set(!need_deleted, XrUI::F_NoDelete);
}


// What the toolbar pushes around its buttons. Shared, because the row's height is
// only honest if it is measured with the same numbers the row is drawn with.
float UIToolBarFramePad()	{ return 2.f; }
float UIToolBarWindowPad()	{ return 1.f; }

// Icons scale with the font: the toolbar is the one row that is all icon, so a
// fixed pixel size makes them shrink to nothing next to scaled-up text. The row
// below is icon + 6 and cannot be shorter, so this number is also the row's floor.
float UIToolBarIconSize()
{
	return _max(24.f, ImGui::GetFontSize() * 1.37f);
}

// What the row actually needs: the icon, the button frame around it, and the
// window padding above and below.
float UIToolBarHeight()
{
	// Measured with the padding the toolbar actually pushes, not the theme's:
	// the default WindowPadding is three times larger, and counting it left a
	// visible strip of nothing under the buttons.
	return UIToolBarIconSize() + UIToolBarFramePad() * 2.f + UIToolBarWindowPad() * 2.f;
}

void XrUIManager::Draw()
{
    // guards the whole frame, so a panel that asks for a forced redraw while
    // drawing gets the deferred one instead of a nested (fatal) ImGui frame
    m_InUIPass = true;

    XUI::Hooks::BeforeNewFrame();
    #if defined(USE_DX11)
    ImGui_ImplDX11_NewFrame();
#else
    ImGui_ImplDX9_NewFrame();
#endif
    NewPlatformFrame();
    ImGui::NewFrame();
    // the frame (caption, toolbar, status bar, the active tab's work area) is drawn by the editor's main form,
    // the first of m_UIArray; the shell routes the keys and frees closed tabs before anything is drawn
    XUI::Shell::BeginFrame();
	for (XrUI* ui : m_UIArray)
	{
		ui->Draw();
	}

    OnDrawUI();
    // the drawer over the work area, the session file
    XUI::Shell::EndFrame();
    // the kit's own dialogs and toasts, above everything the frame drew
    XUI::Hooks::SetProgressFrame(false);
    XUI::DrawOverlays();
    ImGui::EndFrame();
    ImGui::Render();
#if defined(USE_DX11)
    ImGui_ImplDX11_RenderDrawData(ImGui::GetDrawData());
#else
    ImGui_ImplDX9_RenderDrawData(ImGui::GetDrawData());
#endif
    if (ImGui::GetIO().ConfigFlags & ImGuiConfigFlags_ViewportsEnable)
    {
        ImGui::UpdatePlatformWindows();
        ImGui::RenderPlatformWindowsDefault();
    }
    // cleared here rather than at the end: the pruning below makes no ImGui
    // calls and can return early
    m_InUIPass = false;

	for (size_t i = m_UIArray.size(); i > 0; i--)
	{
		if (m_UIArray[i - 1]->IsClosed())
		{
			if (!m_UIArray[i - 1]->Flags.test(XrUI::F_NoDelete))
			{
				xr_delete(m_UIArray[i - 1]);
			}
			m_UIArray.erase(m_UIArray.begin() + (i - 1));
			i = m_UIArray.size();
			if (i == 0)return;
		}
	}
}

void XrUIManager::DrawProgressOnly()
{
	if (m_InUIPass)
		return;

	m_InUIPass = true;
	try
	{
		XUI::Hooks::BeforeNewFrame();
#if defined(USE_DX11)
		ImGui_ImplDX11_NewFrame();
#else
		ImGui_ImplDX9_NewFrame();
#endif
		NewPlatformFrame();
		ImGui::NewFrame();
		OnDrawProgressUI();
		XUI::Hooks::SetProgressFrame(true);
		XUI::DrawOverlays();
		XUI::Hooks::SetProgressFrame(false);
		ImGui::EndFrame();
		ImGui::Render();
#if defined(USE_DX11)
		ImGui_ImplDX11_RenderDrawData(ImGui::GetDrawData());
#else
		ImGui_ImplDX9_RenderDrawData(ImGui::GetDrawData());
#endif
		// Platform windows stay frozen until the operation ends; processing their
		// close/resize requests here would re-enter normal editor window state.
	}
	catch (...)
	{
		m_InUIPass = false;
		throw;
	}
	m_InUIPass = false;
}
static bool ImGui_ImplWin32_UpdateMouseCursor()
{
    ImGuiIO& io = ImGui::GetIO();
    if (io.ConfigFlags & ImGuiConfigFlags_NoMouseCursorChange)
        return false;

    ImGuiMouseCursor imgui_cursor = ImGui::GetMouseCursor();
    if (imgui_cursor == ImGuiMouseCursor_None || io.MouseDrawCursor)
    {
        // Hide OS mouse cursor if imgui is drawing it or if it wants no cursor
        ::SetCursor(NULL);
    }
    else
    {
        // Show OS mouse cursor
        LPTSTR win32_cursor = IDC_ARROW;
        switch (imgui_cursor)
        {
        case ImGuiMouseCursor_Arrow:        win32_cursor = IDC_ARROW; break;
        case ImGuiMouseCursor_TextInput:    win32_cursor = IDC_IBEAM; break;
        case ImGuiMouseCursor_ResizeAll:    win32_cursor = IDC_SIZEALL; break;
        case ImGuiMouseCursor_ResizeEW:     win32_cursor = IDC_SIZEWE; break;
        case ImGuiMouseCursor_ResizeNS:     win32_cursor = IDC_SIZENS; break;
        case ImGuiMouseCursor_ResizeNESW:   win32_cursor = IDC_SIZENESW; break;
        case ImGuiMouseCursor_ResizeNWSE:   win32_cursor = IDC_SIZENWSE; break;
        case ImGuiMouseCursor_Hand:         win32_cursor = IDC_HAND; break;
        case ImGuiMouseCursor_NotAllowed:   win32_cursor = IDC_NO; break;
        }
        ::SetCursor(::LoadCursor(NULL, win32_cursor));
    }
    return true;
}
#ifndef WM_MOUSEHWHEEL
#define WM_MOUSEHWHEEL 0x020E
#endif
#ifndef DBT_DEVNODES_CHANGED
#define DBT_DEVNODES_CHANGED 0x0007
#endif
IMGUI_IMPL_API LRESULT ImGui_ImplWin32_WndProcHandler(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam);
LRESULT XrUIManager::WndProcHandler(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam)
{
    if (msg == WM_DROPFILES)
    {
        XUI::FileDropMessage((void*)wParam);
        return 0;
    }
	if (m_ProgressOnlyInput)
	{
		switch (msg)
		{
		case WM_CLOSE:
		case WM_SIZE:
		case WM_SIZING:
		case WM_ENTERSIZEMOVE:
		case WM_EXITSIZEMOVE:
		case WM_NCLBUTTONDOWN:
		case WM_NCLBUTTONDBLCLK:
		case WM_SYSCOMMAND:
			return 1;
		case WM_KEYDOWN:
		case WM_KEYUP:
		case WM_SYSKEYDOWN:
		case WM_SYSKEYUP:
		case WM_CHAR:
			ImGui_ImplWin32_WndProcHandler(hwnd, msg, wParam, lParam);
			return 1;
		default:
			break;
		}
	}
    switch (msg)
    {
    case WM_DESTROY:
        ::PostQuitMessage(0);
        return 0;
    case WM_KEYDOWN:
    case WM_SYSKEYDOWN:
        switch (wParam)
        {
        case VK_MENU:
        case VK_CONTROL:
        case VK_SHIFT:
            break;
        default:
            if(!IsPlayInEditor() && !IsViewportNavigating())   ApplyShortCut(wParam);
            break;
        }
    default:
        break;
    }
    if (!IsPlayInEditor())
        return ImGui_ImplWin32_WndProcHandler(hwnd, msg, wParam, lParam);;
    return 0;
}
