#pragma once
enum TShiftState_
{
	ssNone = 0,
	ssShift = 1,
	ssLeft = 2,
	ssRight = 4,
	ssCtrl = 8,
	ssAlt = 16,
	ssMiddle = 32,
};
using TShiftState = int;
// The toolbar was 24 pixels flat. That stopped fitting the moment the UI scaled:
// 20px icons plus a button frame do not go into 24 minus padding, so the row was
// cut off at the bottom and the icons stayed tiny however large the font got.
// Both follow the font now, and the dockspace below reserves what they actually take.
XREUI_API float UIToolBarFramePad();
XREUI_API float UIToolBarWindowPad();
XREUI_API float UIToolBarIconSize();
XREUI_API float UIToolBarHeight();
class XREUI_API XrUIManager
{
public:
	XrUIManager();
	void Push(XrUI*ui,bool need_deleted =true);
	void Draw();
	void DrawProgressOnly();
		
	virtual ~XrUIManager();

	LRESULT WndProcHandler(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam);
#if defined(USE_DX11)
	// D3D11 splits device and context; the ImGui backend needs both
	void Initialize(HWND hWnd, ID3D11Device* device, ID3D11DeviceContext* context, const char* ini_path);
#else
	void Initialize(HWND hWnd, IDirect3DDevice9* device,const char*ini_path);
#endif
	void Destroy();

	void ResetBegin();
	void ResetEnd();
	virtual bool 	ApplyShortCut(DWORD Key, TShiftState Shift)=0;

	inline TShiftState GetShiftState()const { return m_ShiftState; };
	virtual bool IsPlayInEditor() { return false; }
	// true while the viewport camera owns the mouse (UE-style fly/pan) —
	// keyboard shortcuts must not fire from WASD etc. during navigation
	virtual bool IsViewportNavigating() { return false; }

	// True between this frame's ImGui NewFrame and Render. An ImGui frame
	// CANNOT nest: a second NewFrame throws away the outer frame's window
	// stack and its EndFrame leaves g.CurrentWindow null, so the panel that
	// was mid-draw dies on its next ImGui call. Panel code that asks for a
	// forced redraw has to be told to wait instead.
	bool InUIPass() const { return m_InUIPass; }
	void SetProgressOnlyInput(bool active) { m_ProgressOnlyInput = active; }

	// The size of the swap chain the UI is drawn into, given before every frame: a
	// minimized window has no client area to take it from (see NewPlatformFrame).
	void SetRenderSize(int width, int height) { m_RenderWidth = width; m_RenderHeight = height; }
	// The modal on top when it is drawn in the editor's own window: its title, its id (what
	// follows ### - dlg.<name> - or the title) and its rect (x, y, w, h) in the pixels of that
	// window's frame. False when there is none.
	bool TopModal(xr_string& title, float rect[4], xr_string* id = nullptr) const;

protected:
	virtual void OnDrawUI();
	virtual void OnDrawProgressUI() {}
private:
	void NewPlatformFrame();
	HWND m_hWnd{nullptr};
	int m_RenderWidth{0};
	int m_RenderHeight{0};
	bool m_InUIPass{false};
	bool m_ProgressOnlyInput{false};
	void ApplyShortCut(DWORD Key);
	TShiftState m_ShiftState{0};
	xr_vector<XrUI*> m_UIArray;
	string_path m_name_ini;
};

