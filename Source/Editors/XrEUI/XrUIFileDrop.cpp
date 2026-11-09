#include "stdafx.h"
#include "XrUIKitHooks.h"
#include "imgui_internal.h"

#include <shellapi.h>
#include <shlobj.h>

// Files dragged in from outside the editor arrive as WM_DROPFILES. An OLE drop target would also say where they are on
// the way, but the editor's main thread is in the multithreaded apartment, where RegisterDragDrop fails.
namespace
{
struct SDrop
{
	HWND			hwnd = nullptr;
	bool			landed = false;
	int				landed_frame = -1;
	bool			taken = false;		// this frame's zones: the first under the cursor has it
	int				taken_frame = -1;
	ImVec2			pos = ImVec2(0.f, 0.f);
	XUI::Strings	files;
};
SDrop s_Drop;
}

void XUI::FileDropInstall(void* hwnd)
{
	s_Drop.hwnd = HWND(hwnd);
	::DragAcceptFiles(s_Drop.hwnd, TRUE);
}

void XUI::FileDropRemove()
{
	if (s_Drop.hwnd)
		::DragAcceptFiles(s_Drop.hwnd, FALSE);
	s_Drop = SDrop();
}

void XUI::FileDropMessage(void* hdrop)
{
	const HDROP drop = HDROP(hdrop);
	s_Drop.files.clear();
	const UINT count = ::DragQueryFileW(drop, 0xFFFFFFFF, nullptr, 0);
	for (UINT i = 0; i < count; ++i)
	{
		wchar_t wide[MAX_PATH * 2] = {};
		if (!::DragQueryFileW(drop, i, wide, UINT(sizeof(wide) / sizeof(wide[0]))))
			continue;
		char utf8[MAX_PATH * 4] = {};
		if (::WideCharToMultiByte(CP_UTF8, 0, wide, -1, utf8, sizeof(utf8), nullptr, nullptr))
			s_Drop.files.push_back(utf8);
	}
	POINT p = {};
	::DragQueryPoint(drop, &p);
	::DragFinish(drop);
	const ImVec2 origin = ImGui::GetMainViewport()->Pos;
	s_Drop.pos = ImVec2(origin.x + float(p.x), origin.y + float(p.y));
	s_Drop.landed = !s_Drop.files.empty();
	s_Drop.landed_frame = -1;
	if (s_Drop.hwnd)
		::PostMessageW(s_Drop.hwnd, WM_NULL, 0, 0);
}

bool XUI::FileDropActive() { return s_Drop.landed; }

// A real HDROP, so a test goes through everything but Windows' delivery of the message.
void XUI::FileDropSimulate(const Strings& files, float x, float y)
{
	std::wstring names;
	for (const xr_string& file : files)
	{
		wchar_t wide[MAX_PATH * 2] = {};
		::MultiByteToWideChar(CP_UTF8, 0, file.c_str(), -1, wide, int(sizeof(wide) / sizeof(wide[0])));
		names += wide;
		names += L'\0';
	}
	names += L'\0';
	const size_t bytes = sizeof(DROPFILES) + names.size() * sizeof(wchar_t);
	const HGLOBAL memory = ::GlobalAlloc(GHND | GMEM_SHARE, bytes);
	if (!memory)
		return;
	DROPFILES* header = static_cast<DROPFILES*>(::GlobalLock(memory));
	header->pFiles = sizeof(DROPFILES);
	header->pt.x = LONG(x);
	header->pt.y = LONG(y);
	header->fWide = TRUE;
	memcpy(header + 1, names.data(), names.size() * sizeof(wchar_t));
	::GlobalUnlock(memory);
	FileDropMessage(memory);
}

bool XUI::FileDragging()
{
	const ImGuiPayload* payload = ImGui::GetDragDropPayload();
	return payload && payload->IsDataType(kAssetPayload);
}

bool XUI::FileDropZone(cstr id, const ImVec2& min, const ImVec2& max, Strings& out)
{
	// a tile dragged out of the content browser: the path it carries
	const ImRect rect(min, max);
	if (ImGui::BeginDragDropTargetCustom(rect, ImGui::GetID(id)))
	{
		bool taken = false;
		if (const ImGuiPayload* payload = ImGui::AcceptDragDropPayload(kAssetPayload))
		{
			out.clear();
			out.push_back(static_cast<const char*>(payload->Data));
			taken = true;
		}
		ImGui::EndDragDropTarget();
		if (taken)
			return true;
	}

	if (!s_Drop.landed)
		return false;
	const int frame = ImGui::GetFrameCount();
	if (s_Drop.taken_frame != frame)
	{
		s_Drop.taken_frame = frame;
		s_Drop.taken = false;
	}
	if (s_Drop.taken || !rect.Contains(s_Drop.pos))
		return false;
	s_Drop.taken = true;
	out = s_Drop.files;
	s_Drop.landed = false;
	s_Drop.files.clear();
	return true;
}

void XUI::Hooks::FileDropFrameEnd()
{
	// nothing took them this frame: they fell where nothing is listening
	if (!s_Drop.landed)
		return;
	if (s_Drop.landed_frame < 0)
		s_Drop.landed_frame = ImGui::GetFrameCount();
	else if (ImGui::GetFrameCount() > s_Drop.landed_frame)
	{
		s_Drop.landed = false;
		s_Drop.files.clear();
	}
}
