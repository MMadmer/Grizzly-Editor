#include "stdafx.h"
#include "EditorStartup.h"
#include "../../Public/xfined_resource.h"

//------------------------------------------------------------------------------
// DPI
//------------------------------------------------------------------------------
ECORE_API float EditorStartup::DpiScale()
{
	// Resolved once: the awareness opt-in only takes on the FIRST call, and a
	// later one answers "access denied" - which would leave the second caller
	// believing the process is unaware and sizing its window at 100%.
	static float scale = 0.f;
	if (scale > 0.f)	return scale;

	scale = 1.f;
	if (HMODULE user32 = ::GetModuleHandleA("user32.dll"))
	{
		typedef BOOL (WINAPI* SetCtxFn)(HANDLE);
		typedef BOOL (WINAPI* SetAwareFn)();
		typedef UINT (WINAPI* GetDpiFn)();
		SetCtxFn	set_ctx		= (SetCtxFn)::GetProcAddress(user32, "SetProcessDpiAwarenessContext");
		SetAwareFn	set_aware	= (SetAwareFn)::GetProcAddress(user32, "SetProcessDPIAware");
		// DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 == (DPI_AWARENESS_CONTEXT)-4
		const bool aware = set_ctx ? !!set_ctx((HANDLE)-4) : (set_aware && set_aware());
		if (aware)
			if (GetDpiFn get_dpi = (GetDpiFn)::GetProcAddress(user32, "GetDpiForSystem"))
				scale = float(get_dpi()) / 96.f;
	}
	return scale;
}

//------------------------------------------------------------------------------
// the splash window
//
// Its own thread on purpose. The main thread spends the whole start inside
// blocking file work; a window pumped from there would stop painting, go grey
// and collect a "not responding" from Windows at the exact moment the user is
// waiting to be told that something IS happening.
//------------------------------------------------------------------------------
namespace
{
const wchar_t	kSplashClass[]	= L"XFinedEditorSplash";
const UINT_PTR	kSweepTimer		= 1;
const int		kSweepPeriod	= 80;		// ticks per pass, 33 ms each

// logical pixels, scaled by the monitor DPI at creation
const int		kWidth			= 460;
const int		kHeight			= 140;
const int		kIcon			= 48;
const int		kBarHeight		= 4;

HANDLE				s_Thread		= 0;
DWORD				s_ThreadId		= 0;
CRITICAL_SECTION	s_TextLock;
bool				s_TextLockReady	= false;
wchar_t				s_Text[160]		= L"Starting";

void TextSet(LPCSTR ansi)
{
	if (!s_TextLockReady)	return;
	::EnterCriticalSection(&s_TextLock);
	if (!ansi || !ansi[0])
		s_Text[0] = 0;
	else if (!::MultiByteToWideChar(CP_ACP, 0, ansi, -1, s_Text, _countof(s_Text)))
		s_Text[0] = 0;
	::LeaveCriticalSection(&s_TextLock);
}

void TextGet(wchar_t* out, int count)
{
	out[0] = 0;
	if (!s_TextLockReady)	return;
	::EnterCriticalSection(&s_TextLock);
	::wcsncpy_s(out, count, s_Text, _TRUNCATE);
	::LeaveCriticalSection(&s_TextLock);
}

struct SSplashState
{
	HFONT	title;
	HFONT	status;
	HICON	icon;
	int		sweep;
	float	scale;
};

void PaintSplash(HWND wnd, SSplashState& st)
{
	PAINTSTRUCT	ps;
	HDC			dc = ::BeginPaint(wnd, &ps);
	RECT		rc;	::GetClientRect(wnd, &rc);

	// double buffered: the sweep repaints thirty times a second
	HDC		mem		= ::CreateCompatibleDC(dc);
	HBITMAP	bmp		= ::CreateCompatibleBitmap(dc, rc.right, rc.bottom);
	HGDIOBJ	old_bmp	= ::SelectObject(mem, bmp);

	const int	pad		= int(28 * st.scale);
	const int	text_x	= pad + int(kIcon * st.scale) + int(16 * st.scale);

	HBRUSH back = ::CreateSolidBrush(RGB(24, 24, 27));
	::FillRect(mem, &rc, back);
	::DeleteObject(back);

	HBRUSH edge = ::CreateSolidBrush(RGB(62, 62, 70));
	::FrameRect(mem, &rc, edge);
	::DeleteObject(edge);

	if (st.icon)
		::DrawIconEx(mem, pad, int(30 * st.scale), st.icon,
			int(kIcon * st.scale), int(kIcon * st.scale), 0, NULL, DI_NORMAL);

	::SetBkMode(mem, TRANSPARENT);

	RECT tr;
	::SetRect(&tr, text_x, int(30 * st.scale), rc.right - pad, int(62 * st.scale));
	::SelectObject(mem, st.title);
	::SetTextColor(mem, RGB(236, 236, 240));
	::DrawTextW(mem, L"XFined Editor", -1, &tr, DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX);

	wchar_t status[160];	TextGet(status, _countof(status));
	::SetRect(&tr, text_x, int(68 * st.scale), rc.right - pad, int(92 * st.scale));
	::SelectObject(mem, st.status);
	::SetTextColor(mem, RGB(146, 146, 156));
	::DrawTextW(mem, status, -1, &tr, DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS);

	// the sweep: no percentage is honest here, because the stages have no
	// comparable size - it says "still working", nothing more
	const int	bar_h	= int(kBarHeight * st.scale);
	RECT		bar;	::SetRect(&bar, 1, rc.bottom - bar_h, rc.right - 1, rc.bottom);
	HBRUSH		track	= ::CreateSolidBrush(RGB(38, 38, 43));
	::FillRect(mem, &bar, track);
	::DeleteObject(track);

	const int	span	= bar.right - bar.left;
	const int	lit		= span / 4;
	const int	x		= bar.left - lit + ((span + lit + lit) * st.sweep) / kSweepPeriod;
	RECT		run;	::SetRect(&run, x < bar.left ? bar.left : x, bar.top,
							  x + lit > bar.right ? bar.right : x + lit, bar.bottom);
	if (run.right > run.left)
	{
		HBRUSH hot = ::CreateSolidBrush(RGB(0, 122, 204));
		::FillRect(mem, &run, hot);
		::DeleteObject(hot);
	}

	::BitBlt(dc, 0, 0, rc.right, rc.bottom, mem, 0, 0, SRCCOPY);
	::SelectObject(mem, old_bmp);
	::DeleteObject(bmp);
	::DeleteDC(mem);
	::EndPaint(wnd, &ps);
}

LRESULT WINAPI SplashProc(HWND wnd, UINT msg, WPARAM wp, LPARAM lp)
{
	SSplashState* st = (SSplashState*)::GetWindowLongPtrW(wnd, GWLP_USERDATA);
	switch (msg)
	{
	case WM_PAINT:
		if (st)	PaintSplash(wnd, *st);
		return 0;
	case WM_TIMER:
		if (st && wp == kSweepTimer)
		{
			st->sweep = (st->sweep + 1) % kSweepPeriod;
			::InvalidateRect(wnd, NULL, FALSE);
		}
		return 0;
	case WM_ERASEBKGND:
		return 1;		// WM_PAINT covers every pixel
	case WM_CLOSE:
		return 0;		// only the editor takes this window down
	}
	return ::DefWindowProcW(wnd, msg, wp, lp);
}

DWORD WINAPI SplashThread(LPVOID)
{
	const float		scale	= EditorStartup::DpiScale();
	const HINSTANCE	self	= ::GetModuleHandleW(NULL);

	WNDCLASSEXW wc		= { sizeof(wc) };
	wc.lpfnWndProc		= SplashProc;
	wc.hInstance		= self;
	// IDC_ARROW is an ordinal, not a string: the editors build /MBCS, so the
	// stock name expands to the ANSI form of a value both entry points take
	wc.hCursor			= ::LoadCursorW(NULL, (LPCWSTR)IDC_ARROW);
	wc.lpszClassName	= kSplashClass;
	::RegisterClassExW(&wc);

	const int w = int(kWidth * scale);
	const int h = int(kHeight * scale);

	// the monitor the pointer is on: the user is looking at the screen they
	// just clicked on, which on a two-screen desk is not always the primary
	POINT		cursor	= {};	::GetCursorPos(&cursor);
	MONITORINFO	mi		= { sizeof(mi) };
	::GetMonitorInfoW(::MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY), &mi);
	const int x = mi.rcWork.left + (mi.rcWork.right - mi.rcWork.left - w) / 2;
	const int y = mi.rcWork.top + (mi.rcWork.bottom - mi.rcWork.top - h) / 2;

	HWND wnd = ::CreateWindowExW(WS_EX_TOPMOST, kSplashClass, L"XFined Editor",
		WS_POPUP, x, y, w, h, NULL, NULL, self, NULL);
	if (!wnd)	return 0;

	SSplashState st;
	st.scale	= scale;
	st.sweep	= 0;
	st.title	= ::CreateFontW(-int(21 * scale), 0, 0, 0, FW_SEMIBOLD, FALSE, FALSE, FALSE,
					DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
					CLEARTYPE_QUALITY, VARIABLE_PITCH, L"Segoe UI");
	st.status	= ::CreateFontW(-int(13 * scale), 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE,
					DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
					CLEARTYPE_QUALITY, VARIABLE_PITCH, L"Segoe UI");
	st.icon		= (HICON)::LoadImageW(self, MAKEINTRESOURCEW(IDI_XFINED_EDITOR), IMAGE_ICON,
					int(kIcon * scale), int(kIcon * scale), LR_SHARED);
	::SetWindowLongPtrW(wnd, GWLP_USERDATA, (LONG_PTR)&st);

	// rounded corners where the system has them; square ones elsewhere
	if (HMODULE dwm = ::GetModuleHandleW(L"dwmapi.dll"))
	{
		typedef HRESULT (WINAPI* SetAttrFn)(HWND, DWORD, LPCVOID, DWORD);
		if (SetAttrFn set_attr = (SetAttrFn)::GetProcAddress(dwm, "DwmSetWindowAttribute"))
		{
			const DWORD round = 2;	// DWMWCP_ROUND
			set_attr(wnd, 33 /*DWMWA_WINDOW_CORNER_PREFERENCE*/, &round, sizeof(round));
		}
	}

	// no activation: the splash is something to look at, not to type into, and
	// stealing focus from whatever the user was doing would be rude
	::ShowWindow(wnd, SW_SHOWNOACTIVATE);
	::UpdateWindow(wnd);
	::SetTimer(wnd, kSweepTimer, 33, NULL);

	MSG msg;
	while (::GetMessageW(&msg, NULL, 0, 0) > 0)
	{
		::TranslateMessage(&msg);
		::DispatchMessageW(&msg);
	}

	::KillTimer(wnd, kSweepTimer);
	::SetWindowLongPtrW(wnd, GWLP_USERDATA, 0);
	::DestroyWindow(wnd);
	::DeleteObject(st.title);
	::DeleteObject(st.status);
	::UnregisterClassW(kSplashClass, self);
	return 0;
}

void SplashClose()
{
	if (!s_Thread)	return;
	// the thread owns the window; WM_QUIT ends its loop and it tears the
	// window down itself, which is the only thread allowed to
	::PostThreadMessageW(s_ThreadId, WM_QUIT, 0, 0);
	::WaitForSingleObject(s_Thread, 2000);
	::CloseHandle(s_Thread);
	s_Thread	= 0;
	s_ThreadId	= 0;
}
} // namespace

//------------------------------------------------------------------------------
// the profile
//------------------------------------------------------------------------------
namespace
{
struct SStage
{
	char	name[64];
	float	seconds;
	u64		reads;		// read calls that reached the file system
	u64		ops;		// opens, closes, directory listings, queries
};

const int		kMaxStages	= 16;
SStage			s_Stages[kMaxStages];
int				s_StageCount	= 0;
char			s_Running[64]	= "";
LARGE_INTEGER	s_Freq			= {};
LARGE_INTEGER	s_RunningAt		= {};
u64				s_RunningReads	= 0;
u64				s_RunningOps	= 0;
float			s_Loader		= 0.f;
u64				s_LoaderReads	= 0;
u64				s_LoaderOps		= 0;
bool			s_Finished		= false;

// What the start costs is the NUMBER of times it touches the file system, not
// the bytes it moves: warm, the whole thing is 26 MB spread over ~19k reads.
// Cold, every one of those is a disk round trip an on-access scanner also gets
// a say in - which is the difference between four seconds and a minute. The
// counters make that visible per stage instead of leaving it to be guessed at.
void IoSample(u64& reads, u64& ops)
{
	IO_COUNTERS io = {};
	if (::GetProcessIoCounters(::GetCurrentProcess(), &io))
	{
		reads	= io.ReadOperationCount;
		ops		= io.OtherOperationCount;
	}
}

// Process creation to now. What comes before the first line of editor code -
// the loader mapping thirty-odd binaries and whatever inspects them on the way
// in - is part of the wait and has to be visible, even though it is not ours.
float SinceProcessStart()
{
	FILETIME created = {}, exited = {}, kernel = {}, user = {}, now = {};
	if (!::GetProcessTimes(::GetCurrentProcess(), &created, &exited, &kernel, &user))
		return 0.f;
	::GetSystemTimeAsFileTime(&now);
	ULARGE_INTEGER c, n;
	c.LowPart = created.dwLowDateTime;	c.HighPart = created.dwHighDateTime;
	n.LowPart = now.dwLowDateTime;		n.HighPart = now.dwHighDateTime;
	return n.QuadPart > c.QuadPart ? float(double(n.QuadPart - c.QuadPart) / 1.0e7) : 0.f;
}

void StageClose()
{
	if (!s_Running[0])	return;
	LARGE_INTEGER now;	::QueryPerformanceCounter(&now);
	u64 reads = 0, ops = 0;	IoSample(reads, ops);
	if (s_StageCount < kMaxStages)
	{
		SStage& s = s_Stages[s_StageCount++];
		::strncpy_s(s.name, s_Running, _TRUNCATE);
		s.seconds = s_Freq.QuadPart
			? float(double(now.QuadPart - s_RunningAt.QuadPart) / double(s_Freq.QuadPart))
			: 0.f;
		s.reads	= reads - s_RunningReads;
		s.ops	= ops - s_RunningOps;
	}
	s_RunningReads	= reads;
	s_RunningOps	= ops;
	s_Running[0]	= 0;
}

// Everything the process did before the first mark belongs to the loader.
void ProfileBegin()
{
	if (s_Freq.QuadPart)	return;
	::QueryPerformanceFrequency(&s_Freq);
	s_Loader = SinceProcessStart();
	IoSample(s_LoaderReads, s_LoaderOps);
	s_RunningReads	= s_LoaderReads;
	s_RunningOps	= s_LoaderOps;
}
} // namespace

ECORE_API void EditorStartup::SplashShow()
{
	if (s_Thread || s_Finished)	return;

	ProfileBegin();

	// the scale is resolved here, on the main thread, so the splash thread and
	// the editor window read the same settled value
	DpiScale();

	::InitializeCriticalSection(&s_TextLock);
	s_TextLockReady = true;

	s_Thread = ::CreateThread(NULL, 0, SplashThread, NULL, 0, &s_ThreadId);
	Stage("Starting");
}

ECORE_API void EditorStartup::Stage(LPCSTR name)
{
	if (s_Finished)	return;
	ProfileBegin();
	StageClose();
	::strncpy_s(s_Running, name ? name : "", _TRUNCATE);
	::QueryPerformanceCounter(&s_RunningAt);
	TextSet(name);
}

ECORE_API void EditorStartup::Finish()
{
	if (s_Finished)	return;
	s_Finished = true;

	StageClose();
	SplashClose();
	if (s_TextLockReady)
	{
		::DeleteCriticalSection(&s_TextLock);
		s_TextLockReady = false;
	}

	const float	total	= SinceProcessStart();
	u64			reads	= 0, ops = 0;
	IoSample(reads, ops);

	// One block, every start, whether it was fast or not: a start that took a
	// minute is only diagnosable against one that did not.
	Msg("* startup: %.1f s, %u file reads, %u file operations", total, u32(reads), u32(ops));
	Msg("*   %-28s %6.2f s  %7u reads %7u ops", "loader (before wWinMain)",
		s_Loader, u32(s_LoaderReads), u32(s_LoaderOps));
	for (int i = 0; i < s_StageCount; ++i)
		Msg("*   %-28s %6.2f s  %7u reads %7u ops", s_Stages[i].name,
			s_Stages[i].seconds, u32(s_Stages[i].reads), u32(s_Stages[i].ops));

	// LevelEditor.manifest asks for UTF-8 narrow APIs; Windows before 1903 ignores
	// that, and a garbled file name is then explained by this line
	const UINT acp = ::GetACP();
	if (acp == CP_UTF8)	Msg("* file names: UTF-8 (ANSI code page %u)", acp);
	else				Msg("! file names: ANSI code page %u, not UTF-8 - a name it cannot hold does not open", acp);
}
