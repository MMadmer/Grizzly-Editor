#include "stdafx.h"
#include "EditorProject.h"
#include "EditorModManifest.h"
#include "EditorUiContract.h"
#include "EditorGameContent.h"
#include "XFinedMCP.h"
#include "UI_ToolsCustom.h"
#include "ui_main.h"
#include "UI_MainCommand.h"
#include "Nq/NqUtil.h"
#include "../XrEngine/XR_IOConsole.h"
#if defined(USE_DX11)
#include "EDX11Utils.h"
// the preview pass flushes the captions the scene queued
#include "d3dutils.h"
// the png encoder itself is compiled into RedImageTool, which XrECore links -
// this only pulls the declarations
#include "StbImage\stb_image_write.h"
// the decoder likewise: a card's preview is looked at before it becomes a texture
#include "StbImage\stb_image.h"
#endif
#include <shobjidl.h>
#include <shellapi.h>
#include <shlobj.h>
#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "shell32.lib")

//------------------------------------------------------------------------------
// state
//------------------------------------------------------------------------------
static const int kMaxRecent = 12;

#if defined(USE_DX11)
// the tile is drawn by ImGui, and ImGui binds a view, not a texture
typedef ID3D11ShaderResourceView	TPreview;
#else
typedef IDirect3DTexture9			TPreview;
#endif

// What a recent entry is now: read when the list loads and again when its menu
// opens. A folder that is gone stays on the list until the author takes it off -
// a drive that is not plugged in is not a project nobody wants any more.
enum ERecentState
{
	rsProject,		// the folder holds project.ltx or mod.ltx
	rsMissing,		// the folder does not exist
	rsNoProject,	// a folder with neither in it
};
// as list_projects and project_browser_menu name them
static const char* kRecentStateNames[] = { "project", "missing", "not_a_project" };

struct SRecentProject
{
	string_path				path;
	string_path				name;
	// What the card draws, worked out when the list loads and never per frame: the name and the path as UTF-8,
	// and for a name another card shares, the nearest parent folder that tells the two apart - with where it
	// and the leaf stand in shown_path (bytes), for the path line that keeps it whole
	xr_string				shown_name;
	xr_string				shown_path;
	xr_string				shown_parent;
	size_t					parent_begin;
	size_t					parent_end;
	size_t					leaf_begin;
	TPreview*				preview;
	u32						preview_w;
	u32						preview_h;
	bool					preview_tried;
	ERecentState			state;
};

static char				s_AppRoot[MAX_PATH]	= {};
static char				s_Project[MAX_PATH]	= {};
static char				s_Name[MAX_PATH]	= {};
static char				s_BaseMaps[MAX_PATH]= {};
static bool				s_Active			= false;
static bool				s_WantImport		= false;
static SRecentProject	s_Recent[kMaxRecent];
static int				s_RecentCount		= 0;
// create-form fields
static char				s_NewParent[MAX_PATH]= {};
static char				s_NewName[64]		= {};
static char				s_PendingScene[MAX_PATH] = {};
// the autoload PopPendingScene handed out, and whether any scene load has
// succeeded since the project opened - see Close()
static char				s_PoppedScene[MAX_PATH] = {};
static bool				s_SceneLoaded		= false;
// the scene file open as of the last frame (FollowScene): a close that clears the scene first still knows it
static char				s_Scene[MAX_PATH]	= {};
static EditorProject::TLiveSceneQuery s_LiveSceneQuery = 0;
static EditorProject::TPreviewScope s_PreviewScope = 0;
static char				s_OpenError[256]		= {};
// ---- linked game install ----------------------------------------------------
static char				s_GameRoot[MAX_PATH]	= {};	// [project] game_root
static char				s_LinkError[256]		= {};	// last LinkGame() failure
// GameLinked() is polled from the draw loop and from every keystroke, so the
// three GetFileAttributes calls behind it are cached for a second. The timer
// also makes a game folder that disappears mid-session surface on its own.
static bool				s_LinkOk				= false;
static u32				s_LinkTick				= 0;
static bool				s_LinkFresh				= false;
// link-dialog fields, plus a cache so the live hint does not stat every frame
static char				s_LinkPath[MAX_PATH]	= {};
static char				s_LinkChecked[MAX_PATH]	= {};
static char				s_LinkHint[256]			= {};
static bool				s_LinkHintOk			= false;
// ---- the Start page ---------------------------------------------------------
static char				s_Search[128]			= {};	// UTF-8, as typed
static int				s_Sort					= 0;	// 0 last opened, 1 name; _projects.ini [browser] sort
static xr_string		s_PageError;					// the last Open / Open Folder failure, UTF-8
static char				s_CreateError[MAX_PATH * 2] = {};
// the create form's look at its target folder, taken again only when a field changes
static char				s_NewChecked[MAX_PATH * 2] = {};
enum ENewState { nsNew, nsProjectThere, nsFolderThere, nsNotFullPath };
static ENewState		s_NewState				= nsNew;
// ---- the card menu and the Delete Project confirmation ------------------------
static string_path		s_MenuFor				= {};	// MCP: put up this card's menu on the next frame
static bool				s_MenuClose				= false;	// MCP: take any card menu down on the next frame
static string_path		s_MenuShown				= {};	// the card whose menu the last frame drew
static string_path		s_ForgetPending			= {};	// Remove from List, applied once the cards are drawn
static bool				s_DeleteWant			= false;	// the confirmation is up; false takes it down
static bool				s_DeleteShown			= false;	// the last frame drew the confirmation
static bool				s_DeleteDone			= false;	// a deletion ran and left something: its result is up
static EditorProject::SDeleteInfo	s_DeletePlan;
static EditorProject::SDeleteInfo	s_DeleteResult;
// previews of cards that left the list or the screen: the frame that drew them may still hold them
static xr_vector<TPreview*>	s_DeadPreviews;
// ---- Import Base Scene: the library is read once, when the dialog opens -------
static bool				s_ImportListed			= false;
static xr_vector<xr_string>	s_ImportScenes;
static xr_vector<u8>	s_ImportInProject;
static char				s_ImportSearch[128]		= {};
static xr_string		s_ImportPick;

// aliases whose writes must land inside the project; second column is the
// project-relative directory they get remounted to
static const char* kMount[][2] =
{
	{ "$maps$",				"rawdata\\levels"	},
	{ "$server_backup$",	"rawdata\\backup"	},
	{ "$game_levels$",		"gamedata\\levels"	},
	{ "$game_spawn$",		"gamedata\\spawns"	},
	{ "$import$",			"import"			},
	{ "$detail_objects$",	"import"			},
	{ "$omotion$",			"import"			},
	{ "$omotions$",			"import"			},
	{ "$smotion$",			"import"			},
	{ "$sbones$",			"import"			},
	{ "$temp$",				"temp"				},
	{ "$logs$",				"logs"				},
	{ "$game_saves$",		"savedgames"		},
	{ "$screenshots$",		"screenshots"		},
};
static const int kMountCount = sizeof(kMount)/sizeof(kMount[0]);
// original alias roots captured before the first mount, for clean unmount
static string_path		s_SavedRoot[kMountCount];
static bool				s_SavedRoots = false;

//------------------------------------------------------------------------------
// helpers
//------------------------------------------------------------------------------
static void ResolveAppRoot()
{
	if (s_AppRoot[0]) return;
	::GetModuleFileNameA(NULL, s_AppRoot, sizeof(s_AppRoot));
	for (int i = 0; i < 4; ++i)
		if (char* p = strrchr(s_AppRoot, '\\')) *p = 0;
	strcat_s(s_AppRoot, "\\");
	sprintf_s(s_BaseMaps, "%srawdata\\levels\\", s_AppRoot);
}

static void RecentIni(char* dst, u32 size)
{
	ResolveAppRoot();
	sprintf_s(dst, size, "%s_projects.ini", s_AppRoot);
}

// A project is a folder with project.ltx - or with mod.ltx alone: project.ltx is this
// machine's bookkeeping (the SDK and game paths, the last scene) and a fresh clone of
// a mod's repository does not carry it. Open() writes it.
static bool HasManifest(const char* folder)
{
	char p[MAX_PATH];
	sprintf_s(p, "%s\\project.ltx", folder);
	if (::GetFileAttributesA(p) != INVALID_FILE_ATTRIBUTES)	return true;
	sprintf_s(p, "%s\\mod.ltx", folder);
	return ::GetFileAttributesA(p) != INVALID_FILE_ATTRIBUTES;
}

static bool DirExists(const char* p)
{
	DWORD a = ::GetFileAttributesA(p);
	return a != INVALID_FILE_ATTRIBUTES && (a & FILE_ATTRIBUTE_DIRECTORY);
}

static ERecentState RecentState(const char* folder)
{
	if (!DirExists(folder))	return rsMissing;
	return HasManifest(folder) ? rsProject : rsNoProject;
}

static bool SamePath(LPCSTR left, LPCSTR right)
{
	if (!left || !right || !left[0] || !right[0]) return false;

	string_path normalized_left = {};
	string_path normalized_right = {};
	const DWORD left_size = ::GetFullPathNameA(left, sizeof(normalized_left), normalized_left, NULL);
	const DWORD right_size = ::GetFullPathNameA(right, sizeof(normalized_right), normalized_right, NULL);
	if (!left_size || left_size >= sizeof(normalized_left) || !right_size || right_size >= sizeof(normalized_right))
		return false;

	for (char* p = normalized_left; *p; ++p) if (*p == '/') *p = '\\';
	for (char* p = normalized_right; *p; ++p) if (*p == '/') *p = '\\';
	while (xr_strlen(normalized_left) > 3 && normalized_left[xr_strlen(normalized_left) - 1] == '\\')
		normalized_left[xr_strlen(normalized_left) - 1] = 0;
	while (xr_strlen(normalized_right) > 3 && normalized_right[xr_strlen(normalized_right) - 1] == '\\')
		normalized_right[xr_strlen(normalized_right) - 1] = 0;
	return 0 == _stricmp(normalized_left, normalized_right);
}

static bool HasLiveScene()
{
	return s_LiveSceneQuery && s_LiveSceneQuery();
}

// A path lives its whole life in the system ANSI codepage: the folder dialog
// produces it, the ini stores it, and the X-Ray filesystem hands it to
// CreateFileA. ImGui, on the other hand, reads UTF-8 - so the very same bytes
// drawn as they are form invalid sequences, each decodes to U+FFFD, and the
// atlas has no glyph for that, which is why "D:\Games\Test" in a Cyrillic
// folder came out as a row of '?'. Nothing that reaches the filesystem may
// change, so the conversion sits exactly at the ImGui boundary and nowhere else.
static void AcpToUtf8(LPCSTR src, char* dst, u32 dst_size)
{
	if (!dst || !dst_size) return;
	dst[0] = 0;
	if (!src || !src[0]) return;
	wchar_t w[MAX_PATH * 2];
	if (::MultiByteToWideChar(CP_ACP, 0, src, -1, w, sizeof(w) / sizeof(w[0])) <= 0 ||
		::WideCharToMultiByte(CP_UTF8, 0, w, -1, dst, (int)dst_size, 0, 0) <= 0)
		strncpy_s(dst, dst_size, src, _TRUNCATE);	// unconvertible: better raw than empty
}

// The way back, for text the user typed into a field. Returns false when the
// result does not fit, so the caller keeps the value it already had rather than
// storing a half path.
static bool Utf8ToAcp(LPCSTR src, char* dst, u32 dst_size)
{
	if (!dst || !dst_size) return false;
	if (!src || !src[0]) { dst[0] = 0; return true; }
	wchar_t w[MAX_PATH * 2];
	if (::MultiByteToWideChar(CP_UTF8, 0, src, -1, w, sizeof(w) / sizeof(w[0])) <= 0) return false;
	return ::WideCharToMultiByte(CP_ACP, 0, w, -1, dst, (int)dst_size, 0, 0) > 0;
}

// The kit's path field over a stored ANSI path: the field edits UTF-8, and the
// store takes the text back only when it changed and converts in full.
static bool PathFieldAcp(LPCSTR id, char* stored, u32 stored_size)
{
	char shown[MAX_PATH * 3];
	AcpToUtf8(stored, shown, sizeof(shown));
	return XUI::PathField(id, shown, sizeof(shown), true) && Utf8ToAcp(shown, stored, stored_size);
}

static bool IsAbsolutePath(LPCSTR p);
static bool SceneInProject(LPCSTR file, xr_string& rel);

static bool PickFolder(char* dest, u32 dest_size, const wchar_t* title)
{
	// nobody is there to answer a native dialog
	if (XUI::Unattended() || XUI::FromMcp()) return false;
	bool ok = false;
	const HRESULT co = ::CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
	IFileDialog* dlg = 0;
	if (SUCCEEDED(::CoCreateInstance(CLSID_FileOpenDialog, NULL, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&dlg))))
	{
		DWORD opts = 0;
		dlg->GetOptions(&opts);
		dlg->SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM);
		dlg->SetTitle(title);
		if (SUCCEEDED(dlg->Show((HWND)ImGui::GetMainViewport()->PlatformHandleRaw)))
		{
			IShellItem* item = 0;
			if (SUCCEEDED(dlg->GetResult(&item)))
			{
				PWSTR w = 0;
				if (SUCCEEDED(item->GetDisplayName(SIGDN_FILESYSPATH, &w)))
				{
					::WideCharToMultiByte(CP_ACP, 0, w, -1, dest, dest_size, 0, 0);
					::CoTaskMemFree(w);
					ok = dest[0] != 0;
				}
				item->Release();
			}
		}
		dlg->Release();
	}
	if (co == S_OK) ::CoUninitialize();
	return ok;
}

bool EditorProject::ValidateName(LPCSTR name)
{
	if (!name || !name[0]) return false;
	for (const char* p = name; *p; ++p)
	{
		const char c = *p;
		const bool ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
						(c >= '0' && c <= '9') || c == '_' || c == '-';
		if (!ok) return false;
	}
	return true;
}

//------------------------------------------------------------------------------
// recents / manifest
//------------------------------------------------------------------------------
// The folder `up` levels above the leaf of `path` ("D:\a\b\Name", 1 -> "b")
static bool AncestorOf(LPCSTR path, int up, xr_string& out)
{
	xr_vector<xr_string> parts;
	xr_string part;
	for (const char* p = path; ; ++p)
	{
		if (!*p || '\\' == *p || '/' == *p)
		{
			if (!part.empty()) parts.push_back(part);
			part.clear();
			if (!*p) break;
			continue;
		}
		part += *p;
	}
	const int at = int(parts.size()) - 1 - up;
	if (at < 0) return false;
	out = parts[at];
	return true;
}

// Where the folder `up` levels above the leaf of a path, and the leaf itself, stand in it (bytes: the
// separators are ASCII in UTF-8 as well)
static bool SegmentRanges(const xr_string& path, int up, size_t& begin, size_t& end, size_t& leaf)
{
	xr_vector<std::pair<size_t, size_t>> parts;
	size_t from = 0;
	for (size_t i = 0; i <= path.size(); ++i)
		if (i == path.size() || '\\' == path[i] || '/' == path[i])
		{
			if (i > from) parts.push_back(std::make_pair(from, i));
			from = i + 1;
		}
	const int at = int(parts.size()) - 1 - up;
	if (at < 0) return false;
	begin = parts[at].first;
	end = parts[at].second;
	leaf = parts.back().first;
	return true;
}

// The card's text: UTF-8 for ImGui, and for a name two cards share, the nearest
// parent folder that differs between them - two DARF-Overnight-Delivery cards
// read "XFinedProjects" and "Temp" instead of the same word twice
static void RefreshCardText()
{
	char buf[MAX_PATH * 3];
	for (int i = 0; i < s_RecentCount; ++i)
	{
		SRecentProject& r = s_Recent[i];
		AcpToUtf8(r.name, buf, sizeof(buf));
		r.shown_name = buf;
		AcpToUtf8(r.path, buf, sizeof(buf));
		r.shown_path = buf;
		r.shown_parent.clear();
		r.parent_begin = r.parent_end = r.leaf_begin = 0;
	}
	for (int i = 0; i < s_RecentCount; ++i)
	{
		SRecentProject& r = s_Recent[i];
		xr_string mine, theirs;
		for (int up = 1; r.shown_parent.empty() && AncestorOf(r.path, up, mine); ++up)
		{
			bool twin = false, differs = true;
			for (int j = 0; j < s_RecentCount; ++j)
			{
				if (j == i || 0 != _stricmp(s_Recent[j].name, r.name)) continue;
				twin = true;
				if (AncestorOf(s_Recent[j].path, up, theirs) && 0 == _stricmp(theirs.c_str(), mine.c_str())) differs = false;
			}
			if (!twin) break;
			if (!differs) continue;
			AcpToUtf8(mine.c_str(), buf, sizeof(buf));
			r.shown_parent = buf;
			if (!SegmentRanges(r.shown_path, up, r.parent_begin, r.parent_end, r.leaf_begin))
				r.parent_begin = r.parent_end = r.leaf_begin = 0;
		}
	}
}

void EditorProject::LoadRecent()
{
	char ini[MAX_PATH];
	RecentIni(ini, sizeof(ini));
	// the old list's previews: the frame being drawn may still hold them
	ReleasePreviews();
	s_RecentCount = 0;
	for (int i = 0; i < kMaxRecent; ++i)
	{
		char key[16];
		sprintf_s(key, "p%d", i);
		string_path v = {};
		::GetPrivateProfileStringA("recent", key, "", v, sizeof(v), ini);
		if (v[0])
		{
			SRecentProject& r = s_Recent[s_RecentCount++];
			xr_strcpy(r.path, v);
			const char* leaf = strrchr(v, '\\');
			xr_strcpy(r.name, leaf ? leaf + 1 : v);
			r.preview = 0;
			r.preview_w = r.preview_h = 0;
			r.preview_tried = false;
			r.state = RecentState(v);
		}
	}
	::GetPrivateProfileStringA("recent", "create_parent", "", s_NewParent, sizeof(s_NewParent), ini);
	if (!s_NewParent[0])
		::ExpandEnvironmentStringsA("%USERPROFILE%\\Documents\\XFinedProjects", s_NewParent, sizeof(s_NewParent));
	char sort[16] = {};
	::GetPrivateProfileStringA("browser", "sort", "", sort, sizeof(sort), ini);
	s_Sort = 0 == _stricmp(sort, "name") ? 1 : 0;
	RefreshCardText();
}

void EditorProject::SaveRecent()
{
	char ini[MAX_PATH];
	RecentIni(ini, sizeof(ini));
	// current project floats to the top, duplicates collapse
	string_path list[kMaxRecent];
	int n = 0;
	if (s_Project[0]) xr_strcpy(list[n++], s_Project);
	for (int i = 0; i < s_RecentCount && n < kMaxRecent; ++i)
		if (0 != _stricmp(s_Recent[i].path, s_Project))
			xr_strcpy(list[n++], s_Recent[i].path);
	for (int i = 0; i < kMaxRecent; ++i)
	{
		char key[16];
		sprintf_s(key, "p%d", i);
		::WritePrivateProfileStringA("recent", key, (i < n) ? list[i] : "", ini);
	}
	::WritePrivateProfileStringA("recent", "create_parent", s_NewParent, ini);
}

void EditorProject::LoadManifest()
{
	char p[MAX_PATH];
	sprintf_s(p, "%s\\project.ltx", s_Project);
	::GetPrivateProfileStringA("project", "game_root", "", s_GameRoot, sizeof(s_GameRoot), p);
	// trailing separators would double up in every <root>\... concatenation
	while (s_GameRoot[0] && (s_GameRoot[xr_strlen(s_GameRoot)-1] == '\\' ||
							 s_GameRoot[xr_strlen(s_GameRoot)-1] == '/'))
		s_GameRoot[xr_strlen(s_GameRoot)-1] = 0;

	s_LinkFresh		= false;		// force a real check on the next GameLinked()
	s_LinkError[0]	= 0;
	strcpy_s(s_LinkPath, s_GameRoot);	// seed the dialog with the stored value
	s_LinkChecked[0]= 0;
}

void EditorProject::SaveManifest()
{
	char p[MAX_PATH];
	sprintf_s(p, "%s\\project.ltx", s_Project);
	::WritePrivateProfileStringA("project", "name", s_Name, p);
	::WritePrivateProfileStringA("project", "sdk_root", s_AppRoot, p);
	::WritePrivateProfileStringA("project", "game_root", s_GameRoot, p);
	// last_scene is written by Close() alone, from the scene the session had open
}

//------------------------------------------------------------------------------
// linked game install
//------------------------------------------------------------------------------
LPCSTR	EditorProject::GameRoot()	{ return s_GameRoot; }
LPCSTR	EditorProject::LinkError()	{ return s_LinkError; }
void EditorProject::SetLiveSceneQuery(TLiveSceneQuery query) { s_LiveSceneQuery = query; }

bool EditorProject::GameLinked()
{
	if (!s_GameRoot[0]) return false;

	const u32 now = ::GetTickCount();
	if (s_LinkFresh && (now - s_LinkTick) < 1000) return s_LinkOk;

	xr_string reason;
	s_LinkOk	= EditorGameContent::LooksLikeGame(s_GameRoot, reason);
	s_LinkTick	= now;
	s_LinkFresh	= true;
	return s_LinkOk;
}

bool EditorProject::LinkGame(LPCSTR folder)
{
	s_LinkError[0] = 0;
	if (!s_Active)
	{
		strcpy_s(s_LinkError, "no project is open");
		return false;
	}

	char clean[MAX_PATH] = {};
	strncpy_s(clean, sizeof(clean), folder ? folder : "", _TRUNCATE);
	for (char* p = clean; *p; ++p) if (*p == '/') *p = '\\';
	while (clean[0] && clean[xr_strlen(clean)-1] == '\\')
		clean[xr_strlen(clean)-1] = 0;
	if (SamePath(s_GameRoot, clean))
	{
		xr_string reason;
		if (EditorGameContent::LooksLikeGame(clean, reason)) return true;
		strncpy_s(s_LinkError, sizeof(s_LinkError), reason.c_str(), _TRUNCATE);
		return false;
	}
	if (HasLiveScene())
	{
		strcpy_s(s_LinkError, "close or clear the current scene before linking a different game");
		return false;
	}

	xr_string reason;
	if (!EditorGameContent::LooksLikeGame(clean, reason))
	{
		strncpy_s(s_LinkError, sizeof(s_LinkError), reason.c_str(), _TRUNCATE);
		return false;
	}

	xr_string err;
	if (!EditorGameContent::Mount(clean, err))
	{
		strncpy_s(s_LinkError, sizeof(s_LinkError), err.c_str(), _TRUNCATE);
		return false;
	}

	strcpy_s(s_GameRoot, clean);
	s_LinkFresh = false;
	SaveManifest();

	Msg("* Game linked: %s", s_GameRoot);
	return true;
}

void EditorProject::McpGameLink(LPCSTR raw, xr_string& out)
{
	char path[MAX_PATH] = {};
	const bool want_link = XFinedMCP::GetArg(raw, "path", path, sizeof(path)) && path[0];

	bool ok = true;
	xr_string reason;
	if (!s_Active)
	{
		ok		= false;
		reason	= "no project is open";
	}
	else if (want_link)
	{
		for (char* p = path; *p; ++p) if (*p == '/') *p = '\\';
		ok = LinkGame(path);
		if (!ok) reason = s_LinkError;
	}
	else if (s_GameRoot[0])
	{
		EditorGameContent::LooksLikeGame(s_GameRoot, reason);
	}
	else reason = "no game linked";

	// paths travel with forward slashes, same as the rest of the MCP layer
	xr_string root;
	for (const char* p = s_GameRoot; *p; ++p) root += (*p == '\\') ? '/' : *p;

	out  = ok ? "{\"ok\":true,\"linked\":" : "{\"ok\":false,\"linked\":";
	out += (s_Active && GameLinked()) ? "true" : "false";
	out += ",\"game_root\":\"";
	out += root;
	out += "\",\"reason\":\"";
	out += reason;
	out += "\"}";
}

//------------------------------------------------------------------------------
// mount / unmount
//------------------------------------------------------------------------------
void EditorProject::CreateLayout()
{
	// Nothing. A project is born EMPTY - the author decides its layout, and no
	// folder exists until something is actually put in it. The write path
	// (ELocatorAPI::w_open -> VerifyPath) creates parent chains on demand, so
	// the editor's own scratch (temp, logs, import, ...) appears when first
	// written to, and a module's folders when the author makes them.
}

// The editor's scratch, path bookkeeping and captured preview. Everything NOT
// on this list is the author's mod: any folder they create in the project root
// is module content by definition ([vfs] can publish it anywhere), so the
// content browser shows it and the export ships it.
// releases\ holds packaged releases (Mod > Package Release): build output that
// must never travel inside the module it was built from.
bool EditorProject::IsEditorOnlyEntry(LPCSTR n)
{
	static const char* editor_only[] =
	{
		"import", "temp", "logs", "savedgames", "screenshots", "backup", "releases",
		"project.ltx", "preview.png",
	};
	for (int i = 0; i < (int)(sizeof(editor_only)/sizeof(editor_only[0])); ++i)
		if (0 == _stricmp(n, editor_only[i])) return true;
	return false;
}

// Author-facing sources the game itself never reads: scenes and .object
// sources under rawdata, the legacy Content folder. Shown and editable, but
// the module export leaves them out.
bool EditorProject::IsSourceOnlyEntry(LPCSTR n)
{
	return 0 == _stricmp(n, "rawdata") || 0 == _stricmp(n, "Content");
}

bool EditorProject::IsToolingEntry(LPCSTR n)
{
	if (!n || !n[0])	return false;
	// A leading dot is how every tool marks its own - .git, .gitignore, .github,
	// .vscode, .idea, .vs, .svn, .DS_Store, .editorconfig - and nothing the game
	// reads is named that way.
	if ('.' == n[0])	return true;
	// what the Windows shell leaves in folders it has shown
	static const char* shell[] = { "Thumbs.db", "ehthumbs.db", "ehthumbs_vista.db", "desktop.ini" };
	for (int i = 0; i < (int)(sizeof(shell) / sizeof(shell[0])); ++i)
		if (0 == _stricmp(n, shell[i]))	return true;
	// an Office document open elsewhere keeps a lock file beside it
	return '~' == n[0] && '$' == n[1];
}

// A project that is a git working copy gets the editor's own files ignored, ONCE:
// the scratch and the packaged releases change with every session and every build,
// and a repository that tracks them fills up with zip files, crash leftovers and one
// machine's paths. Written only when there is no .gitignore at all - an author's own
// is never touched, and this one is theirs to edit from then on.
static void EnsureGitIgnore(LPCSTR root)
{
	string_path git, ignore;
	sprintf_s(git, "%s\\.git", root);
	sprintf_s(ignore, "%s\\.gitignore", root);
	// a folder in a plain clone, a file in a worktree or a submodule: both mean git
	if (INVALID_FILE_ATTRIBUTES == ::GetFileAttributesA(git))	return;
	if (INVALID_FILE_ATTRIBUTES != ::GetFileAttributesA(ignore))	return;

	static const char* text =
		"# XFined Editor: its own files, never part of the mod. Written once because\n"
		"# this project is a git repository - edit it freely, the editor will not again.\n"
		"\n"
		"# this machine's paths and last scene; the editor recreates it on open\n"
		"/project.ltx\n"
		"/preview.png\n"
		"# packaged releases - rebuilt from the sources, published on GitHub\n"
		"/releases/\n"
		"# scratch\n"
		"/backup/\n"
		"/temp/\n"
		"/logs/\n"
		"/import/\n"
		"/savedgames/\n"
		"/screenshots/\n"
		"\n"
		"# author key files: never published, never shipped (Mod > Submit to Mod Browser)\n"
		"*.xkey\n"
		"\n"
		"# Windows\n"
		"Thumbs.db\n"
		"desktop.ini\n";
	// CREATE_NEW: a file that appeared in the meantime is the author's, not ours
	HANDLE h = ::CreateFileA(ignore, GENERIC_WRITE, 0, NULL, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, NULL);
	if (INVALID_HANDLE_VALUE == h)	return;
	DWORD written = 0;
	::WriteFile(h, text, (DWORD)xr_strlen(text), &written, NULL);
	::CloseHandle(h);
	Msg("* Project is a git repository: .gitignore written for the editor's own files");
}

void EditorProject::Mount()
{
	for (int i = 0; i < kMountCount; ++i)
	{
		FS_Path* P = FS.get_path(kMount[i][0]);
		if (!P) continue;
		if (!s_SavedRoots) xr_strcpy(s_SavedRoot[i], P->m_Path);

		string_path root;
		sprintf_s(root, "%s\\%s", s_Project, kMount[i][1]);
		P->_set_root(root);
		P->_set("");	// path = root exactly, whatever the original add part was
	}
	s_SavedRoots = true;
}

void EditorProject::Unmount()
{
	if (!s_SavedRoots) return;
	for (int i = 0; i < kMountCount; ++i)
	{
		FS_Path* P = FS.get_path(kMount[i][0]);
		if (!P) continue;
		P->_set_root(s_SavedRoot[i]);
		P->_set("");
	}
}

//------------------------------------------------------------------------------
// open / create / close
//------------------------------------------------------------------------------
bool EditorProject::Open(LPCSTR folder)
{
	s_OpenError[0] = 0;
	if (!folder || !DirExists(folder))
	{
		strcpy_s(s_OpenError, "project folder does not exist");
		return false;
	}
	if (s_Active && SamePath(s_Project, folder)) return true;
	if (s_Active && HasLiveScene())
	{
		strcpy_s(s_OpenError, "close or clear the current scene before switching projects");
		return false;
	}
	if (s_Active) Close();
	ResolveAppRoot();

	xr_strcpy(s_Project, folder);
	const char* leaf = strrchr(s_Project, '\\');
	xr_strcpy(s_Name, leaf ? leaf + 1 : s_Project);
	s_PoppedScene[0] = 0;
	s_SceneLoaded = false;
	s_Scene[0] = 0;

	CreateLayout();		// deliberately a no-op: a project's layout is the author's
	// XMS: silently add mod.ltx + module dirs to projects that predate them
	EditorMod::EnsureScaffold(s_Project, s_Name);
	LoadManifest();
	Mount();
	EnsureGitIgnore(s_Project);
	// typing in the browser may have woken the in-engine console — put it away
	if (Console) Console->Hide();
	SaveManifest();
	SaveRecent();
	// the cards go off screen; their previews are released a frame later, because
	// a click on a card runs this inside the draw that still holds them
	ReleasePreviews();
	s_Search[0] = 0;
	s_PageError.clear();
	s_Active = true;

	// remember the last scene for a DEFERRED load — running COMMAND_LOAD here
	// would execute a heavy scene load inside the browser's popup stack
	char manifest[MAX_PATH], stored[MAX_PATH] = {};
	sprintf_s(manifest, "%s\\project.ltx", s_Project);
	::GetPrivateProfileStringA("project", "last_scene", "", stored, sizeof(stored), manifest);
	s_PendingScene[0] = 0;
	if (stored[0])
	{
		// project-relative; an absolute path counts only inside this folder - a copy of a project must never open,
		// and then save into, the scene of the project it was copied from
		const xr_string file = IsAbsolutePath(stored) ? xr_string(stored) : xr_string(s_Project) + "\\" + stored;
		xr_string rel;
		if (!SceneInProject(file.c_str(), rel))
			Msg("~ project: the last scene '%s' lies outside the project and is not opened", stored);
		else
		{
			const xr_string inside = xr_string(s_Project) + "\\" + rel;
			// a stale path is skipped silently
			if (inside.size() < sizeof(s_PendingScene) && ::GetFileAttributesA(inside.c_str()) != INVALID_FILE_ATTRIBUTES)
				strcpy_s(s_PendingScene, inside.c_str());
		}
	}

	Msg("* Project opened: %s", s_Project);
	return true;
}

LPCSTR EditorProject::OpenError() { return s_OpenError; }

void EditorProject::ListRecentJson(xr_string& out)
{
	char ini[MAX_PATH];
	RecentIni(ini, sizeof(ini));
	out = "[";
	bool first = true;
	for (int i = 0; i < kMaxRecent; ++i)
	{
		char key[16];
		sprintf_s(key, "p%d", i);
		string_path v = {};
		::GetPrivateProfileStringA("recent", key, "", v, sizeof(v), ini);
		if (!v[0]) continue;
		// every entry the browser shows a tile for, a folder that is gone included
		const char* leaf = strrchr(v, '\\');
		out += first ? "{\"name\":\"" : ",{\"name\":\"";
		EditorGameContent::JsonAppendPath(out, leaf ? leaf + 1 : v);
		out += "\",\"path\":\"";
		EditorGameContent::JsonAppendPath(out, v);
		out += "\",\"state\":\"";
		out += kRecentStateNames[RecentState(v)];
		out += "\"}";
		first = false;
	}
	out += "]";
}

LPCSTR EditorProject::PopPendingScene()
{
	if (!s_Active || !s_PendingScene[0]) return 0;
	// a window that started minimized gets here late: a scripted caller may have
	// loaded a scene meanwhile, and the autoload must not replace it
	if (HasLiveScene())
	{
		s_PendingScene[0] = 0;
		return 0;
	}
	static char out[MAX_PATH];
	strcpy_s(out, s_PendingScene);
	strcpy_s(s_PoppedScene, s_PendingScene);
	s_PendingScene[0] = 0;
	// crash insurance: clear last_scene BEFORE loading, so a load crash can't
	// lock the project out; a successful session rewrites it on close
	char manifest[MAX_PATH];
	sprintf_s(manifest, "%s\\project.ltx", s_Project);
	::WritePrivateProfileStringA("project", "last_scene", "", manifest);
	return out;
}

bool EditorProject::Create(LPCSTR parent, LPCSTR name)
{
	s_CreateError[0] = 0;
	if (!ValidateName(name))
	{
		strcpy_s(s_CreateError, "The name may hold Latin letters, digits, _ and - only.");
		return false;
	}
	// a relative location would land in the editor's working folder
	if (!IsAbsolutePath(parent))
	{
		strcpy_s(s_CreateError, "The location must be a full path.");
		return false;
	}
	if (xr_strlen(parent) + xr_strlen(name) + 2 > MAX_PATH)
	{
		strcpy_s(s_CreateError, "The path is too long.");
		return false;
	}

	char folder[MAX_PATH];
	sprintf_s(folder, "%s\\%s", parent, name);
	if (!(DirExists(folder) && HasManifest(folder)))
	{
		// create the whole chain (parent may not exist yet)
		char acc[MAX_PATH] = {};
		for (const char* p = folder;; ++p)
		{
			if (*p == '\\' || *p == 0)
			{
				if (acc[0] && acc[xr_strlen(acc)-1] != ':') ::CreateDirectoryA(acc, NULL);
				if (*p == 0) break;
			}
			acc[p - folder] = *p;
			acc[p - folder + 1] = 0;
		}
		if (!DirExists(folder))
		{
			sprintf_s(s_CreateError, "The folder cannot be created: %s (win32 %lu).", folder, ::GetLastError());
			return false;
		}
		EditorMod::EnsureScaffold(folder, name);	// XMS module skeleton
	}
	// a project there already is simply opened
	if (Open(folder)) return true;
	strcpy_s(s_CreateError, s_OpenError);
	return false;
}

void EditorProject::NoteSceneLoaded()
{
	s_SceneLoaded = true;
	if (Tools) strncpy_s(s_Scene, Tools->m_LastFileName.c_str(), _TRUNCATE);
}

// Once a frame, after the frame's work. A close that clears the scene before it closes the project
// (project_close, Switch Project) does both within one frame, so what this saw last is the scene
// that was open; a scene the author closed on a frame of its own is gone from here too.
void EditorProject::FollowScene()
{
	if (!s_Active || !Tools) return;
	if (0 != xr_strcmp(Tools->m_LastFileName.c_str(), s_Scene))
		strncpy_s(s_Scene, Tools->m_LastFileName.c_str(), _TRUNCATE);
}

// What the next Open loads, stored below the project root: a copy of the folder opens its own copy.
// In order: the scene open as the project closes; the autoload's, when it never came to anything
// (last_scene is cleared before it runs so that a crash in it cannot lock the project out, and a
// pending recovery refusing it or a broken file failing it is no such crash); the one the autoload
// never reached (closed on the Link Game card). A scene the author closed stays forgotten: that was
// a choice. A scene from outside the project is not reopened.
static void WriteLastScene()
{
	xr_string rel;
	if (s_Scene[0] && !SceneInProject(s_Scene, rel))
		Msg("~ project: the scene '%s' lies outside the project and is not reopened", s_Scene);
	if (rel.empty())
	{
		LPCSTR earlier = (!s_SceneLoaded && s_PoppedScene[0]) ? s_PoppedScene : s_PendingScene;
		if (earlier[0]) SceneInProject(earlier, rel);
	}
	char manifest[MAX_PATH];
	sprintf_s(manifest, "%s\\project.ltx", s_Project);
	// nothing to reopen in a project that never named a scene: its file stays as it was, no empty key added
	if (rel.empty())
	{
		char stored[4] = {};
		::GetPrivateProfileStringA("project", "last_scene", "\x7f", stored, sizeof(stored), manifest);
		if (!strcmp(stored, "\x7f"))
			return;
	}
	::WritePrivateProfileStringA("project", "last_scene", rel.c_str(), manifest);
}

void EditorProject::Close()
{
	if (!s_Active) return;
	// a scene still up (the editor quitting) is the truth; one a close cleared first is what the last frame saw
	if (Tools && Tools->m_LastFileName.size())
		strncpy_s(s_Scene, Tools->m_LastFileName.c_str(), _TRUNCATE);
	// a scene cleared first had its picture taken by whoever cleared it
	if (HasLiveScene()) CapturePreview();
	SaveManifest();
	WriteLastScene();
	s_PoppedScene[0] = 0;
	s_PendingScene[0] = 0;
	s_Scene[0] = 0;
	SaveRecent();
	Unmount();
	EditorGameContent::Unmount();	// closes the archive handles of the old link
	s_Active = false;
	s_Project[0] = 0;
	s_Name[0] = 0;
	s_GameRoot[0] = 0;
	s_LinkFresh = false;
	s_LinkError[0] = 0;
	LoadRecent();	// refresh tiles for the browser that is about to reappear
}

bool	EditorProject::Active()			{ return s_Active; }
LPCSTR	EditorProject::Root()			{ return s_Project; }
LPCSTR	EditorProject::Name()			{ return s_Name; }
LPCSTR	EditorProject::BaseMapsDir()	{ ResolveAppRoot(); return s_BaseMaps; }

static bool CopyTree(const char* src, const char* dst);

void EditorProject::ListBaseScenes(xr_vector<xr_string>& out)
{
	ResolveAppRoot();
	string_path mask;
	sprintf_s(mask, "%s*.level", s_BaseMaps);
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (INVALID_HANDLE_VALUE == h) return;
	do
	{
		if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) continue;
		string_path name;
		strcpy_s(name, fd.cFileName);
		if (char* dot = strrchr(name, '.')) *dot = 0;
		if (name[0]) out.push_back(name);
	} while (::FindNextFileA(h, &fd));
	::FindClose(h);
	std::sort(out.begin(), out.end());
}

bool EditorProject::HasBaseScene(LPCSTR name)
{
	if (!name || !name[0]) return false;
	ResolveAppRoot();
	string_path f;
	sprintf_s(f, "%s%s.level", s_BaseMaps, name);
	const DWORD a = ::GetFileAttributesA(f);
	return a != INVALID_FILE_ATTRIBUTES && !(a & FILE_ATTRIBUTE_DIRECTORY);
}

bool EditorProject::ImportBaseScene(LPCSTR name, string_path& out_level, xr_string& err)
{
	err.clear();
	out_level[0] = 0;
	if (!s_Active)					{ err = "no active project"; return false; }
	if (!name || !name[0])			{ err = "no scene name"; return false; }
	ResolveAppRoot();

	string_path src_file, dst_file, src_dir, dst_dir, dst_root;
	sprintf_s(src_file, "%s%s.level", s_BaseMaps, name);
	sprintf_s(src_dir,  "%s%s", s_BaseMaps, name);
	sprintf_s(dst_root, "%s\\rawdata\\levels", s_Project);
	sprintf_s(dst_file, "%s\\%s.level", dst_root, name);
	sprintf_s(dst_dir,  "%s\\%s", dst_root, name);

	// already in the project: that copy is the author's, leave it alone
	if (INVALID_FILE_ATTRIBUTES != ::GetFileAttributesA(dst_file))
	{
		xr_strcpy(out_level, sizeof(string_path), dst_file);
		return true;
	}
	if (INVALID_FILE_ATTRIBUTES == ::GetFileAttributesA(src_file))
	{
		err = xr_string("no such scene in the editor library: ") + name;
		return false;
	}

	VerifyPath(dst_file);	// the whole chain: an empty project has no rawdata yet
	if (!::CopyFileA(src_file, dst_file, FALSE))
	{
		err = xr_string("cannot copy the scene into the project: ") + name;
		return false;
	}
	if (DirExists(src_dir)) CopyTree(src_dir, dst_dir);

	xr_strcpy(out_level, sizeof(string_path), dst_file);
	Msg("* project: scene '%s' imported from the editor library", name);
	return true;
}

void EditorProject::RequestImportScene()	{ s_WantImport = true; }

void EditorProject::OpenProjectFolder()
{
	ResolveAppRoot();
	::ShellExecuteA(NULL, "open", s_Active ? s_Project : s_AppRoot, NULL, NULL, SW_SHOWNORMAL);
}

//------------------------------------------------------------------------------
// previews
//------------------------------------------------------------------------------
extern ENGINE_API BOOL g_bRendering;

// No picture of a scene: the share of the most common colour (4 bits a channel) over a sample grid.
// A target nothing was rendered into is one flat colour, an empty scene one grey (an older editor wrote
// it with the stats text over it: 99.5% measured), a scene some 6%. The channel order does not matter,
// alpha is left out.
static bool ShowsNoScene(const u32* px, u32 w, u32 h, u32 pitch)
{
	if (!px || !w || !h) return true;
	const u32 step = _max(1u, u32(sqrtf(float(w) * float(h) / 65536.f)));
	u32 bins[4096] = {};
	u32 samples = 0, top = 0;
	for (u32 y = 0; y < h; y += step)
		for (u32 x = 0; x < w; x += step)
		{
			const u32 c = px[size_t(y) * pitch + x];
			top = _max(top, ++bins[((c >> 12) & 0xf00) | ((c >> 8) & 0xf0) | ((c >> 4) & 0xf)]);
			++samples;
		}
	return u64(top) * 50 >= u64(samples) * 49;
}

void EditorProject::SetPreviewScope(TPreviewScope scope) { s_PreviewScope = scope; }

#if defined(USE_DX11)
// The scene once more from the viewport's camera and at its size, into a target of its own. The viewport's
// frame is no picture of the project: it carries the author's tools. So this is the scene pass of TUI::Redraw
// less the grid and pivot, the selection rectangle, the axis triad, the stats and the captions, drawn solid;
// the host editor takes the selection and the gizmo away for it (SetPreviewScope). Rows come back top-down,
// B8G8R8A8. Outside a frame and outside the UI pass, like the thumbnail queue: the next frame binds its own.
static bool RenderPreview(U32Vec& px, u32& w, u32& h)
{
	const u32 want_w = UI->GetRenderWidth(), want_h = UI->GetRenderHeight();
	// a viewport folded down to a sliver pictures nothing
	if (want_w < 64 || want_h < 64) return false;
	SDX11Target target;
	if (!target.create(want_w, want_h)) return false;

	// what the pass changes goes back after it: the device's targets and viewport last, so declared first
	SDX11TargetGuard	targets;
	const SEditorFixedFunc ff = EDevice->ff;
	const Fmatrix		world = RCache.get_xform_world(), view = RCache.get_xform_view(), project = RCache.get_xform_project();
	const Fmatrix		device_view = EDevice->mView, device_project = EDevice->mProject, device_full = EDevice->mFullTransform;
	const BOOL			rendering = g_bRendering;

	bool drawn = false;
	if (s_PreviewScope) s_PreviewScope(true);
	try
	{
		// the cache forgets what it thinks is bound; the device is told the truth right after
		RCache.Invalidate();
		target.bind();
		target.clear(EPrefs ? EPrefs->scene_clear_color : 0x0);
		RCache.set_RT(target.rtv);
		RCache.set_ZB(target.dsv);
		g_bRendering = TRUE;

		EDevice->mProject.build_projection(deg2rad(EDevice->fFOV), EDevice->fASPECT, EDevice->m_Camera._Znear(),
			EDevice->m_Camera._Zfar());
		RCache.set_xform_project(EDevice->mProject);
		EDevice->UpdateView();
		EDevice->ResetMaterial();
		// wireframe is a way of looking at the scene, not the scene
		EDevice->SetRS(D3DRS_FILLMODE, D3DFILL_SOLID);
		Tools->RenderEnvironment();
		const u32 filter = psDeviceFlags.is(rsFilterLinear) ? D3DTEXF_LINEAR : D3DTEXF_POINT;
		for (u32 k = 0; k < HW.Caps.raster.dwStages; k++)
		{
			EDevice->SetSS(k, D3DSAMP_MAGFILTER, filter);
			EDevice->SetSS(k, D3DSAMP_MINFILTER, filter);
			EDevice->SetSS(k, D3DSAMP_MIPFILTER, filter);
		}
		Tools->Render();
		drawn = DX11ReadbackBGRA(target.rt, px, w, h);
		// the captions the scene queued (spawn names, way points) go into this target and away with it,
		// not onto the next frame
		DU_impl.OnRender();
	}
	catch (...)
	{
		Msg("! project: the preview pass failed, preview.png is kept as it was");
		drawn = false;
	}
	if (s_PreviewScope) s_PreviewScope(false);

	g_bRendering = rendering;
	EDevice->ff = ff;
	EDevice->SetRS(D3DRS_FILLMODE, EDevice->dwFillMode);
	RCache.set_xform_world(world);
	RCache.set_xform_view(view);
	RCache.set_xform_project(project);
	EDevice->mView = device_view;
	EDevice->mProject = device_project;
	EDevice->mFullTransform = device_full;
	// the pass set state behind the cache's back: drop it all, the next draw applies its own
	RCache.OnFrameEnd();
	return drawn && w && h;
}
#endif

bool EditorProject::CapturePreview()
{
	if (!s_Active || !HW.pDevice || !UI || !Tools || !HasLiveScene()) return false;
	// the viewport shows the game then, not the scene
	if (UI->IsPlayInEditor()) return false;
	if (UI->InUIPass())
	{
		Msg("! project: the preview is never rendered inside the UI pass, preview.png is kept as it was");
		return false;
	}
#if defined(USE_DX11)
	U32Vec	px;
	u32		w = 0, h = 0;
	if (!RenderPreview(px, w, h)) return false;
	// a camera looking at nothing is no picture of the project: the card keeps its last good one
	if (ShowsNoScene(px.data(), w, h, w))
	{
		Msg("* project: the scene shows nothing from the viewport's camera, preview.png is kept as it was");
		return false;
	}
	// stb writes bytes in memory order: swap R and B, force the picture opaque
	for (u32& c : px)
		c = 0xff000000 | (c & 0x0000ff00) | ((c & 0x00ff0000) >> 16) | ((c & 0x000000ff) << 16);
	char p[MAX_PATH];
	sprintf_s(p, "%s\\preview.png", s_Project);
	if (!stbi_write_png(p, int(w), int(h), 4, px.data(), int(w * sizeof(u32))))
	{
		Msg("! project: preview.png cannot be written: %s", p);
		return false;
	}
	return true;
#else
	return false;
#endif
}

// the size comes along: the card crops the picture to its own shape
#if defined(USE_DX11)
// a card's picture is some 650 px wide at 3x: a whole viewport's worth of texture per card only costs memory
static const u32 kPreviewMaxW = 640;

// Averaged down to kPreviewMaxW. A picture of no scene (a black or empty grey frame an older editor
// kept) gives the card its placeholder; the file itself is the project's and stays.
static ID3D11ShaderResourceView* LoadPreview(const char* project, u32& w, u32& h)
{
	w = h = 0;
	char p[MAX_PATH];
	sprintf_s(p, "%s\\preview.png", project);
	int iw = 0, ih = 0, channels = 0;
	stbi_uc* rgba = stbi_load(p, &iw, &ih, &channels, 4);
	if (!rgba) return 0;
	const u32 f = _max(1u, u32(iw) / kPreviewMaxW), ow = u32(iw) / f, oh = u32(ih) / f, n = f * f;
	U32Vec px(size_t(ow) * oh);
	for (u32 y = 0; y < oh; ++y)
		for (u32 x = 0; x < ow; ++x)
		{
			u32 sum[3] = {};
			for (u32 sy = 0; sy < f; ++sy)
			{
				const stbi_uc* src = rgba + ((size_t(y) * f + sy) * u32(iw) + size_t(x) * f) * 4;
				for (u32 sx = 0; sx < f; ++sx, src += 4)
					for (int c = 0; c < 3; ++c) sum[c] += src[c];
			}
			// B8G8R8A8, opaque
			px[size_t(y) * ow + x] = 0xff000000 | ((sum[0] / n) << 16) | ((sum[1] / n) << 8) | (sum[2] / n);
		}
	stbi_image_free(rgba);
	if (!ow || !oh || ShowsNoScene(px.data(), ow, oh, ow)) return 0;
	ID3D11ShaderResourceView* view = DX11TextureFromPixels(px.data(), ow, oh);
	if (!view) return 0;
	w = ow;
	h = oh;
	return view;
}
#else
static IDirect3DTexture9* LoadPreview(const char* project, u32& w, u32& h)
{
	w = h = 0;
	char p[MAX_PATH];
	sprintf_s(p, "%s\\preview.png", project);
	if (::GetFileAttributesA(p) == INVALID_FILE_ATTRIBUTES) return 0;
	IDirect3DTexture9* tex = 0;
	if (FAILED(D3DXCreateTextureFromFileExA(HW.pDevice, p, D3DX_DEFAULT, D3DX_DEFAULT, 1, 0,
		D3DFMT_X8R8G8B8, D3DPOOL_MANAGED, D3DX_FILTER_LINEAR, D3DX_FILTER_NONE, 0, NULL, NULL, &tex)))
		return 0;
	D3DSURFACE_DESC desc;
	if (SUCCEEDED(tex->GetLevelDesc(0, &desc)))
	{
		w = desc.Width;
		h = desc.Height;
	}
	// a picture of no scene gives the card its placeholder
	D3DLOCKED_RECT locked;
	if (w && h && SUCCEEDED(tex->LockRect(0, &locked, NULL, D3DLOCK_READONLY)))
	{
		const bool none = ShowsNoScene((const u32*)locked.pBits, w, h, u32(locked.Pitch) / sizeof(u32));
		tex->UnlockRect(0);
		if (none)
		{
			tex->Release();
			w = h = 0;
			return 0;
		}
	}
	return tex;
}
#endif

// Hands the cards' previews to s_DeadPreviews, released at the start of the next
// frame: the frame being drawn may still hold them, and a renderer left with a
// dangling texture killed the editor on the spot once
void EditorProject::ReleasePreviews()
{
	for (int i = 0; i < s_RecentCount; ++i)
	{
		if (s_Recent[i].preview) s_DeadPreviews.push_back(s_Recent[i].preview);
		s_Recent[i].preview = 0;
		s_Recent[i].preview_tried = false;
	}
}

static void ReleaseDeadPreviews()
{
	for (u32 i = 0; i < s_DeadPreviews.size(); ++i)
		s_DeadPreviews[i]->Release();
	s_DeadPreviews.clear();
}

//------------------------------------------------------------------------------
// the tile menu: Show in Explorer, Delete, Remove from List
//
// A deletion is final - no Recycle Bin, like every deletion of the editor - so
// one set of checks stands in front of it, the same for the confirmation, the
// deletion itself and MCP: only a listed folder that is a project, never the
// open one, never a link, never a drive root or a folder that is or holds the
// user's own folders, the editor or a game, never one inside the editor or a
// game, and never one that holds another project. Inside, a link goes as a
// link and is never followed, and the manifests go last: a deletion that stops
// half way leaves a project that can be deleted again.
//------------------------------------------------------------------------------
// X:\... or \\server\share\... - a relative path would be read against the
// process's working directory, which is nobody's project
static bool IsAbsolutePath(LPCSTR p)
{
	if (!p || !p[0]) return false;
	if (('\\' == p[0] || '/' == p[0]) && ('\\' == p[1] || '/' == p[1])) return true;
	return 0 != isalpha(u8(p[0])) && ':' == p[1] && ('\\' == p[2] || '/' == p[2]);
}

// separators, "." and ".." resolved; no trailing separator but a drive root's
static xr_string FullPath(LPCSTR path)
{
	char buf[MAX_PATH * 2];
	const DWORD n = ::GetFullPathNameA(path, sizeof(buf), buf, NULL);
	if (!n || n >= sizeof(buf)) return xr_string();
	for (char* p = buf; *p; ++p) if ('/' == *p) *p = '\\';
	xr_string out = buf;
	while (out.size() > 3 && '\\' == out.back()) out.pop_back();
	return out;
}

// Where a folder really is - links, junctions and 8.3 names resolved - so two
// spellings of one place compare equal. Empty when it cannot be opened.
static xr_string RealPath(LPCSTR path)
{
	HANDLE h = ::CreateFileA(path, FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
		NULL, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, NULL);
	if (INVALID_HANDLE_VALUE == h) return xr_string();
	char buf[MAX_PATH * 4];
	const DWORD n = ::GetFinalPathNameByHandleA(h, buf, sizeof(buf), FILE_NAME_NORMALIZED | VOLUME_NAME_DOS);
	::CloseHandle(h);
	if (!n || n >= sizeof(buf)) return xr_string();
	xr_string out = buf;
	if (0 == out.compare(0, 8, "\\\\?\\UNC\\"))	out = "\\\\" + out.substr(8);
	else if (0 == out.compare(0, 4, "\\\\?\\"))	out.erase(0, 4);
	while (out.size() > 3 && '\\' == out.back()) out.pop_back();
	return out;
}

// `path` is `folder` or lies below it; both normalised
static bool SameOrBelow(const xr_string& path, const xr_string& folder)
{
	if (path.empty() || folder.empty() || path.size() < folder.size())	return false;
	if (0 != _strnicmp(path.c_str(), folder.c_str(), folder.size()))		return false;
	return path.size() == folder.size() || '\\' == path[folder.size()] || '\\' == folder.back();
}

// `file` lies inside the open project: `rel` is its path below the root ("rawdata\levels\x.level").
// As spelled first, then where both really are (a junction, an 8.3 name).
static bool SceneInProject(LPCSTR file, xr_string& rel)
{
	rel.clear();
	if (!file || !file[0] || !s_Project[0]) return false;
	for (int real = 0; real < 2; ++real)
	{
		const xr_string path = real ? RealPath(file) : FullPath(file);
		const xr_string root = real ? RealPath(s_Project) : FullPath(s_Project);
		if (path.size() <= root.size() || !SameOrBelow(path, root)) continue;
		rel = path.substr(root.size() + ('\\' == root.back() ? 0 : 1));
		return !rel.empty();
	}
	return false;
}

static LPCSTR LeafOf(LPCSTR path)
{
	LPCSTR cut = strrchr(path, '\\');
	return cut ? cut + 1 : path;
}

// the folders _projects.ini names, spelled as it has them
static void ListedFolders(xr_vector<xr_string>& out)
{
	char ini[MAX_PATH];
	RecentIni(ini, sizeof(ini));
	for (int i = 0; i < kMaxRecent; ++i)
	{
		char key[16];
		sprintf_s(key, "p%d", i);
		string_path v = {};
		::GetPrivateProfileStringA("recent", key, "", v, sizeof(v), ini);
		if (v[0]) out.push_back(v);
	}
}

static bool FindListed(LPCSTR path, xr_string& listed)
{
	if (!IsAbsolutePath(path)) return false;
	xr_vector<xr_string> all;
	ListedFolders(all);
	for (u32 i = 0; i < all.size(); ++i)
		if (SamePath(all[i].c_str(), path))
		{
			listed = all[i];
			return true;
		}
	return false;
}

// Takes a folder off the list: the file and the tiles alike
static void ForgetRecent(LPCSTR path)
{
	char ini[MAX_PATH];
	RecentIni(ini, sizeof(ini));
	xr_vector<xr_string> all, keep;
	ListedFolders(all);
	for (u32 i = 0; i < all.size(); ++i)
		if (!SamePath(all[i].c_str(), path)) keep.push_back(all[i]);
	for (int i = 0; i < kMaxRecent; ++i)
	{
		char key[16];
		sprintf_s(key, "p%d", i);
		::WritePrivateProfileStringA("recent", key, i < int(keep.size()) ? keep[i].c_str() : "", ini);
	}

	int kept = 0;
	for (int i = 0; i < s_RecentCount; ++i)
	{
		if (SamePath(s_Recent[i].path, path))
		{
			if (s_Recent[i].preview) s_DeadPreviews.push_back(s_Recent[i].preview);
			continue;
		}
		if (kept != i) s_Recent[kept] = s_Recent[i];
		++kept;
	}
	s_RecentCount = kept;
	// a twin that left takes the need for a parent on the other card with it
	RefreshCardText();
}

// A place no deletion may take with it: the folder itself and anything holding
// it, and for the editor and the games anything inside it as well
struct SGuard
{
	xr_string	path;
	xr_string	real;
	xr_string	what;
	bool		inside;
};

static void AddGuard(xr_vector<SGuard>& guards, LPCSTR path, const xr_string& what, bool inside)
{
	if (!IsAbsolutePath(path)) return;
	SGuard g;
	g.path = FullPath(path);
	if (g.path.empty()) return;
	g.real		= RealPath(g.path.c_str());
	g.what		= what;
	g.inside	= inside;
	guards.push_back(g);
}

static void AddKnownFolder(xr_vector<SGuard>& guards, REFKNOWNFOLDERID id, LPCSTR what)
{
	PWSTR wide = 0;
	if (SUCCEEDED(::SHGetKnownFolderPath(id, 0, 0, &wide)) && wide)
	{
		char path[MAX_PATH * 2];
		if (::WideCharToMultiByte(CP_ACP, 0, wide, -1, path, sizeof(path), 0, 0) > 0)
			AddGuard(guards, path, what, false);
	}
	if (wide) ::CoTaskMemFree(wide);
}

static void GatherGuards(xr_vector<SGuard>& guards)
{
	ResolveAppRoot();
	char exe[MAX_PATH] = {};
	::GetModuleFileNameA(NULL, exe, sizeof(exe));
	if (char* cut = strrchr(exe, '\\')) *cut = 0;
	AddGuard(guards, exe, "the editor", true);
	AddGuard(guards, s_AppRoot, "the editor", true);
	AddKnownFolder(guards, FOLDERID_Profile, "your user profile");
	AddKnownFolder(guards, FOLDERID_Documents, "your Documents folder");
	AddKnownFolder(guards, FOLDERID_Desktop, "your Desktop");
	AddKnownFolder(guards, FOLDERID_Downloads, "your Downloads folder");
	AddKnownFolder(guards, FOLDERID_LocalAppData, "your AppData folder");
	AddKnownFolder(guards, FOLDERID_RoamingAppData, "your AppData folder");
	AddKnownFolder(guards, FOLDERID_Windows, "the Windows folder");
	AddKnownFolder(guards, FOLDERID_ProgramFiles, "Program Files");
	AddKnownFolder(guards, FOLDERID_ProgramFilesX86, "Program Files (x86)");
	char temp[MAX_PATH + 1] = {};
	if (::GetTempPathA(sizeof(temp), temp)) AddGuard(guards, temp, "the Temp folder", false);
	if (s_Active && s_GameRoot[0])
		AddGuard(guards, s_GameRoot, "the game the open project is linked to", true);

	// what every listed project points at: its game, its editor, where it builds to
	xr_vector<xr_string> listed;
	ListedFolders(listed);
	for (u32 i = 0; i < listed.size(); ++i)
	{
		const xr_string manifest = listed[i] + "\\project.ltx";
		if (INVALID_FILE_ATTRIBUTES == ::GetFileAttributesA(manifest.c_str())) continue;
		const xr_string of = xr_string(" of the project ") + LeafOf(listed[i].c_str());
		string_path v;
		::GetPrivateProfileStringA("project", "game_root", "", v, sizeof(v), manifest.c_str());
		AddGuard(guards, v, "the linked game" + of, true);
		::GetPrivateProfileStringA("project", "sdk_root", "", v, sizeof(v), manifest.c_str());
		AddGuard(guards, v, "the editor" + of, true);
		::GetPrivateProfileStringA("xms", "export_target", "", v, sizeof(v), manifest.c_str());
		AddGuard(guards, v, "the export target" + of, true);
	}
}

// A junction or a symbolic link points at somebody's real folder. Other reparse
// points - a OneDrive placeholder - are ordinary files and folders.
static bool IsLink(DWORD attributes, DWORD tag)
{
	return (attributes & FILE_ATTRIBUTE_REPARSE_POINT) && IsReparseTagNameSurrogate(tag);
}

static bool IsManifestName(LPCSTR name)
{
	return 0 == _stricmp(name, "project.ltx") || 0 == _stricmp(name, "mod.ltx");
}

struct SDirEntry
{
	xr_string	name;
	DWORD		attributes;
	bool		link;
	u64			size;
};

// One folder's entries, read in full before anything in it is touched. False when
// the listing does not reach its end: a folder read half way hides what is there.
static bool ListFolder(const xr_string& dir, xr_vector<SDirEntry>& out, DWORD& error)
{
	out.clear();
	error = 0;
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA((dir + "\\*").c_str(), &fd);
	if (INVALID_HANDLE_VALUE == h)
	{
		error = ::GetLastError();
		return false;
	}
	do
	{
		if (!strcmp(fd.cFileName, ".") || !strcmp(fd.cFileName, "..")) continue;
		SDirEntry e;
		e.name			= fd.cFileName;
		e.attributes	= fd.dwFileAttributes;
		e.link			= IsLink(fd.dwFileAttributes, fd.dwReserved0);
		e.size			= (u64(fd.nFileSizeHigh) << 32) | fd.nFileSizeLow;
		out.push_back(e);
	} while (::FindNextFileA(h, &fd));
	error = ::GetLastError();
	::FindClose(h);
	if (ERROR_NO_MORE_FILES != error) return false;
	error = 0;
	return true;
}

// a crashed packaging leaves its stage in temp\ - a copy of the module, no project
static bool IsReleaseStage(const xr_string& root, const xr_string& dir)
{
	static const char stage[] = "\\temp\\release_stage_";
	const size_t n = sizeof(stage) - 1;
	return dir.size() > root.size() + n && 0 == _strnicmp(dir.c_str() + root.size(), stage, n);
}

struct SScan
{
	EditorProject::SDeleteInfo&	info;
	xr_string					root;
	xr_string					unreadable;
	DWORD						error;
};

// counts what the folder holds and finds the projects of their own below it;
// a link is counted, never entered
static bool ScanFolder(SScan& s, const xr_string& dir, bool in_releases)
{
	xr_vector<SDirEntry> entries;
	DWORD error = 0;
	if (!ListFolder(dir, entries, error))
	{
		s.unreadable	= dir;
		s.error			= error;
		return false;
	}
	++s.info.folders;
	const bool root = dir.size() == s.root.size();
	bool manifest = false;
	for (u32 i = 0; i < entries.size(); ++i)
	{
		const SDirEntry& e = entries[i];
		if (e.link)
		{
			++s.info.links;
			continue;
		}
		if (e.attributes & FILE_ATTRIBUTE_DIRECTORY)
		{
			const bool releases = in_releases || (root && 0 == _stricmp(e.name.c_str(), "releases"));
			if (root && 0 == _stricmp(e.name.c_str(), ".git"))	s.info.git = true;
			if (root && releases)								s.info.releases = true;
			if (!ScanFolder(s, dir + "\\" + e.name, releases))	return false;
			continue;
		}
		++s.info.files;
		s.info.bytes += e.size;
		if (in_releases)
		{
			++s.info.release_files;
			s.info.release_bytes += e.size;
		}
		if (!root && IsManifestName(e.name.c_str())) manifest = true;
	}
	if (manifest && !IsReleaseStage(s.root, dir))
	{
		++s.info.nested_total;
		if (s.info.nested.size() < 16) s.info.nested.push_back(dir);
	}
	return true;
}

static xr_string Win32Text(const xr_string& what, DWORD code)
{
	char tail[32];
	sprintf_s(tail, " (win32 %lu)", code);
	return what + tail;
}

static xr_string FailReason(DWORD code)
{
	switch (code)
	{
	case ERROR_SHARING_VIOLATION:
	case ERROR_LOCK_VIOLATION:	return Win32Text("in use by another program", code);
	case ERROR_ACCESS_DENIED:	return Win32Text("access denied", code);
	case ERROR_DIR_NOT_EMPTY:	return Win32Text("the folder is not empty", code);
	default:					return Win32Text("cannot be deleted", code);
	}
}

static void NoteFailure(EditorProject::SDeleteInfo& info, const xr_string& path, const xr_string& reason)
{
	++info.failed_total;
	if (info.failed.size() >= 32) return;
	EditorProject::SDeleteFailure f;
	f.path		= path;
	f.reason	= reason;
	info.failed.push_back(f);
}

// The read-only mark goes first (git keeps its objects read-only) - on the entry
// itself, never on what a link points at
static void MakeWritable(const xr_string& path, DWORD attributes, bool link)
{
	if (INVALID_FILE_ATTRIBUTES == attributes || !(attributes & FILE_ATTRIBUTE_READONLY)) return;
	const DWORD settable = FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM | FILE_ATTRIBUTE_ARCHIVE |
						   FILE_ATTRIBUTE_TEMPORARY | FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED;
	DWORD keep = attributes & settable & ~FILE_ATTRIBUTE_READONLY;
	if (!keep) keep = FILE_ATTRIBUTE_NORMAL;
	if (!link)
	{
		::SetFileAttributesA(path.c_str(), keep);
		return;
	}
	HANDLE h = ::CreateFileA(path.c_str(), FILE_WRITE_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
		NULL, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS, NULL);
	if (INVALID_HANDLE_VALUE == h) return;
	FILE_BASIC_INFO basic = {};
	basic.FileAttributes = keep;
	::SetFileInformationByHandle(h, FileBasicInfo, &basic, sizeof(basic));
	::CloseHandle(h);
}

// A scanner or an indexer that opened a file a moment ago lets go of it shortly,
// and a folder whose last entry is still being let go of reads as not empty
static bool DeleteEntry(const xr_string& path, bool folder, DWORD& error)
{
	for (int attempt = 0;; ++attempt)
	{
		if (folder ? ::RemoveDirectoryA(path.c_str()) : ::DeleteFileA(path.c_str())) return true;
		error = ::GetLastError();
		if (ERROR_FILE_NOT_FOUND == error || ERROR_PATH_NOT_FOUND == error) return true;
		const bool passing = ERROR_SHARING_VIOLATION == error || ERROR_LOCK_VIOLATION == error ||
							 ERROR_ACCESS_DENIED == error || (folder && ERROR_DIR_NOT_EMPTY == error);
		if (!passing || attempt >= 5) return false;
		::Sleep(30);
	}
}

// Empties `dir` and removes it. At the root the manifests are handed back and the
// folder stays: the caller finishes both once everything else is gone.
static bool WipeFolder(const xr_string& dir, DWORD dir_attributes, EditorProject::SDeleteInfo& info,
					   xr_vector<SDirEntry>* root_manifests)
{
	xr_vector<SDirEntry> entries;
	DWORD error = 0;
	if (!ListFolder(dir, entries, error))
	{
		NoteFailure(info, dir, Win32Text("cannot be read", error));
		return false;
	}
	bool all = true;
	for (u32 i = 0; i < entries.size(); ++i)
	{
		const SDirEntry& e = entries[i];
		const xr_string path = dir + "\\" + e.name;
		const bool folder = 0 != (e.attributes & FILE_ATTRIBUTE_DIRECTORY);
		if (root_manifests && !folder && !e.link && IsManifestName(e.name.c_str()))
		{
			root_manifests->push_back(e);
			continue;
		}
		if (folder && !e.link)
		{
			if (!WipeFolder(path, e.attributes, info, 0)) all = false;
			continue;
		}
		MakeWritable(path, e.attributes, e.link);
		if (DeleteEntry(path, folder, error))
		{
			if (e.link)	++info.links;
			else
			{
				++info.files;
				info.bytes += e.size;
			}
			continue;
		}
		NoteFailure(info, path, e.link ? Win32Text("the link cannot be removed", error) : FailReason(error));
		all = false;
	}
	if (root_manifests)	return all;
	// what stayed inside is named already, and keeps this folder from going anyway
	if (!all)			return false;
	MakeWritable(dir, dir_attributes, false);
	if (DeleteEntry(dir, true, error))
	{
		++info.folders;
		return true;
	}
	NoteFailure(info, dir, FailReason(error));
	return false;
}

bool EditorProject::CheckDelete(LPCSTR path, SDeleteInfo& info)
{
	info = SDeleteInfo();
	info.path = path ? path : "";
	if (!IsAbsolutePath(path))
	{
		info.refused = "Name the project by the full path of its folder.";
		return false;
	}
	xr_string listed;
	const bool on_list = FindListed(path, listed);
	info.path = on_list ? listed : FullPath(path);
	info.name = LeafOf(info.path.c_str());

	// gone already: the list entry is all there is to remove
	const DWORD attributes = ::GetFileAttributesA(info.path.c_str());
	if (INVALID_FILE_ATTRIBUTES == attributes)
	{
		if (!on_list)
		{
			info.refused = "There is no such folder, and it is not in the project list.";
			return false;
		}
		info.missing = true;
		return true;
	}
	if (!(attributes & FILE_ATTRIBUTE_DIRECTORY))
	{
		info.refused = "It is a file, not a project folder.";
		return false;
	}
	// a link: the folder it points at is deleted - if at all - where it really is
	WIN32_FIND_DATAA self;
	HANDLE find = ::FindFirstFileA(info.path.c_str(), &self);
	const bool link = INVALID_HANDLE_VALUE != find ? IsLink(self.dwFileAttributes, self.dwReserved0)
												   : 0 != (attributes & FILE_ATTRIBUTE_REPARSE_POINT);
	if (INVALID_HANDLE_VALUE != find) ::FindClose(find);
	if (link)
	{
		const xr_string target = RealPath(info.path.c_str());
		if (target.empty())	info.refused = "The project folder is a link whose target cannot be reached. Remove the link where it is.";
		else				info.refused = "The project folder is a link to " + target + ". Delete the project where it really is.";
		return false;
	}
	if (!HasManifest(info.path.c_str()))
	{
		info.refused = "It holds no project.ltx or mod.ltx - it is not a project.";
		return false;
	}
	if (!on_list)
	{
		info.refused = "It is not in the project list - only a listed project is deleted from here.";
		return false;
	}

	const xr_string full = FullPath(info.path.c_str());
	const xr_string real = RealPath(info.path.c_str());
	if (s_Active && (SamePath(s_Project, info.path.c_str()) || (!real.empty() && 0 == _stricmp(real.c_str(), RealPath(s_Project).c_str()))))
	{
		info.refused = "It is the open project. Close it first: File > Project > Switch Project...";
		return false;
	}
	char volume[MAX_PATH * 2] = {};
	if (::GetVolumePathNameA(full.c_str(), volume, sizeof(volume)))
	{
		xr_string root = volume;
		while (root.size() > 3 && '\\' == root.back()) root.pop_back();
		if (0 == _stricmp(root.c_str(), full.c_str()) || (!real.empty() && 0 == _stricmp(root.c_str(), real.c_str())))
		{
			info.refused = "It is the root of a drive.";
			return false;
		}
	}
	xr_vector<SGuard> guards;
	GatherGuards(guards);
	for (u32 i = 0; i < guards.size(); ++i)
	{
		const SGuard& g = guards[i];
		const bool known = !real.empty() && !g.real.empty();
		const bool same = 0 == _stricmp(g.path.c_str(), full.c_str()) || (known && 0 == _stricmp(g.real.c_str(), real.c_str()));
		const bool holds = SameOrBelow(g.path, full) || (known && SameOrBelow(g.real, real));
		const bool within = g.inside && (SameOrBelow(full, g.path) || (known && SameOrBelow(real, g.real)));
		if (same || holds)
		{
			info.refused = (same ? "It is " : "It holds ") + g.what + ": " + g.path + ".";
			return false;
		}
		if (within)
		{
			info.refused = "It lies inside " + g.what + ": " + g.path + ".";
			return false;
		}
	}

	SScan scan = { info, info.path, xr_string(), 0 };
	if (!ScanFolder(scan, info.path, false))
	{
		info.refused = Win32Text("A folder inside cannot be read: " + scan.unreadable, scan.error) + ".";
		return false;
	}
	if (info.nested_total)
	{
		char head[96];
		if (1 == info.nested_total)	sprintf_s(head, "It holds another project: ");
		else						sprintf_s(head, "It holds %u other projects, among them ", info.nested_total);
		info.refused = head + info.nested[0] + ".";
		return false;
	}
	return true;
}

bool EditorProject::DeleteProject(LPCSTR path, SDeleteInfo& info, xr_string& err)
{
	err.clear();
	if (!CheckDelete(path, info))
	{
		err = info.refused;
		return false;
	}
	if (info.missing)
	{
		ForgetRecent(info.path.c_str());
		info.removed_from_list = true;
		Msg("* Project list: %s removed, the folder does not exist any more", info.path.c_str());
		return true;
	}

	// the check counted what is there; from here on the numbers are what went
	SDeleteInfo done;
	done.path			= info.path;
	done.name			= info.name;
	done.git			= info.git;
	done.releases		= info.releases;
	done.release_files	= info.release_files;
	done.release_bytes	= info.release_bytes;
	const DWORD root_attributes = ::GetFileAttributesA(info.path.c_str());
	xr_vector<SDirEntry> manifests;
	DWORD error = 0;
	if (WipeFolder(info.path, root_attributes, done, &manifests))
	{
		for (u32 i = 0; i < manifests.size(); ++i)
		{
			const xr_string file = info.path + "\\" + manifests[i].name;
			MakeWritable(file, manifests[i].attributes, false);
			if (DeleteEntry(file, false, error))
			{
				++done.files;
				done.bytes += manifests[i].size;
			}
			else NoteFailure(done, file, FailReason(error));
		}
		if (!done.failed_total)
		{
			MakeWritable(info.path, root_attributes, false);
			if (DeleteEntry(info.path, true, error))	++done.folders;
			else										NoteFailure(done, info.path, FailReason(error));
		}
	}
	// never a success for a folder that is still there
	if (!done.failed_total && INVALID_FILE_ATTRIBUTES != ::GetFileAttributesA(info.path.c_str()))
		NoteFailure(done, info.path, "the folder is still there");
	info = done;

	if (info.failed_total)
	{
		char tmp[160];
		sprintf_s(tmp, "Not deleted completely: %u %s stayed behind, so the project stays in the list.",
			info.failed_total, 1 == info.failed_total ? "entry" : "entries");
		err = tmp;
		Msg("! Project %s not deleted completely: %u entries stayed behind, the first: %s (%s)", info.path.c_str(),
			info.failed_total, info.failed[0].path.c_str(), info.failed[0].reason.c_str());
		return false;
	}
	ForgetRecent(info.path.c_str());
	info.removed_from_list = true;
	Msg("* Project deleted: %s (%u files, %u folders, %u links)", info.path.c_str(), info.files, info.folders, info.links);
	return true;
}

bool EditorProject::ForgetProject(LPCSTR path, xr_string& err)
{
	xr_string listed;
	if (!FindListed(path, listed))
	{
		err = "It is not in the project list.";
		return false;
	}
	if (rsProject == RecentState(listed.c_str()))
	{
		err = "The folder still holds a project, so it stays in the list.";
		return false;
	}
	ForgetRecent(listed.c_str());
	return true;
}

// the folder Show in Explorer opens: a listed one that is there
static bool RevealTarget(LPCSTR path, xr_string& folder, xr_string& err)
{
	if (!FindListed(path, folder))
	{
		err = "It is not in the project list - only a listed project is shown from here.";
		return false;
	}
	if (!DirExists(folder.c_str()))
	{
		err = "The folder does not exist any more.";
		return false;
	}
	return true;
}

bool EditorProject::RevealProject(LPCSTR path, xr_string& err)
{
	xr_string folder;
	if (!RevealTarget(path, folder, err)) return false;
	while (folder.size() > 3 && '\\' == folder.back()) folder.pop_back();
	// quoted, a path with spaces being the normal case; a drive root needs no quotes
	// and would read its own trailing separator as an escaped quote
	xr_string args = folder;
	if ('\\' != folder.back()) args = "\"" + folder + "\"";
	const INT_PTR code = (INT_PTR)::ShellExecuteA(NULL, "open", "explorer.exe", args.c_str(), NULL, SW_SHOWNORMAL);
	if (code > 32) return true;
	char tmp[96];
	sprintf_s(tmp, "Windows did not start Explorer (code %d).", int(code));
	err = tmp;
	return false;
}

//------------------------------------------------------------------------------
// the Start page's pieces: text, the page and its cards, a menu row
//------------------------------------------------------------------------------
using CT = XFinedTheme::ColorToken;

// pt, docs/UI_DESIGN.md section 6 (Start page)
static const float kFormW		= 320.f;
static const float kCardW		= 232.f;
static const float kCardH		= 184.f;
static const float kCardGap		= 16.f;
static const float kPreviewH	= 120.f;
static const char* kSortLabels[] = { "Last Opened", "Name" };
static const char* kSortKeys[]	= { "recent", "name" };

static float Snap(float v) { return floorf(v + .5f); }

static void SecondaryText(LPCSTR text)
{
	ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::U32(CT::TextSecondary));
	ImGui::TextUnformatted(text);
	ImGui::PopStyleColor();
}

static void HeadlineText(LPCSTR text)
{
	XUI::FontScope strong(XUI::Font::Strong);
	ImGui::PushTextWrapPos(0.f);
	ImGui::TextUnformatted(text);
	ImGui::PopTextWrapPos();
}

// one line of a list of what goes
static void WhatGoes(LPCSTR text)
{
	ImGui::Indent(XUI::Pt(8.f));
	XUI::TextEllipsis(text, -1.f, XUI::Clip::Middle);
	ImGui::Unindent(XUI::Pt(8.f));
}

static LPCSTR StateHelp(ERecentState state)
{
	return rsMissing == state
		? "The folder is not there: moved, deleted, or on a drive that is not plugged in. Its menu takes it off the list."
		: "The folder holds no project.ltx or mod.ltx any more. Its menu takes it off the list.";
}

static xr_string SizeText(u64 bytes)
{
	char tmp[64];
	if (bytes < 1024)				sprintf_s(tmp, "%llu bytes", bytes);
	else if (bytes < (1ull << 20))	sprintf_s(tmp, "%.1f KB", double(bytes) / 1024.0);
	else if (bytes < (1ull << 30))	sprintf_s(tmp, "%.1f MB", double(bytes) / 1048576.0);
	else							sprintf_s(tmp, "%.2f GB", double(bytes) / 1073741824.0);
	return tmp;
}

static xr_string CountText(u32 n, LPCSTR one, LPCSTR many)
{
	char tmp[64];
	sprintf_s(tmp, "%u %s", n, 1 == n ? one : many);
	return tmp;
}

// "Title###dlg.name" -> "dlg.name", the name XUI::CloseDialog takes
static LPCSTR DialogName(LPCSTR title_id)
{
	LPCSTR hash = strstr(title_id, "###");
	return hash ? hash + 3 : title_id;
}

//------------------------------------------------------------------------------
// the card menu and the Delete Project confirmation
//------------------------------------------------------------------------------
// The cards in the order the page shows them: the search filters, the sort orders
static void CardOrder(xr_vector<int>& out)
{
	out.clear();
	xr_string text;
	for (int i = 0; i < s_RecentCount; ++i)
	{
		text = s_Recent[i].shown_name + " " + s_Recent[i].shown_path;
		if (XUI::Match(s_Search, text.c_str())) out.push_back(i);
	}
	if (1 == s_Sort)
		std::stable_sort(out.begin(), out.end(), [](int a, int b) { return _stricmp(s_Recent[a].name, s_Recent[b].name) < 0; });
}

static void OpenDeleteConfirm(LPCSTR path)
{
	// counted once, when it opens
	EditorProject::CheckDelete(path, s_DeletePlan);
	s_DeleteResult	= EditorProject::SDeleteInfo();
	s_DeleteDone	= false;
	s_DeleteWant	= true;
}

static void DrawCardMenu(SRecentProject& r)
{
	if (!ImGui::BeginPopup("##card_menu"))
		return;
	if (s_MenuClose)
	{
		ImGui::CloseCurrentPopup();
		ImGui::EndPopup();
		return;
	}
	// the folder may have gone since the list was read
	if (ImGui::IsWindowAppearing()) r.state = RecentState(r.path);
	strncpy_s(s_MenuShown, sizeof(s_MenuShown), r.path, _TRUNCATE);

	const bool missing = rsMissing == r.state;
	const bool reveal = XUI::MenuItem(EditorUiContract::kCardShowInExplorer, XUI::Icon::Reveal, nullptr, !missing);
	XUI::Tip(missing ? "The folder does not exist any more." : "Opens the project folder in Windows Explorer.");
	if (reveal)
	{
		xr_string err;
		if (!EditorProject::RevealProject(r.path, err)) s_PageError = "Show in Explorer: " + err;
	}
	ImGui::Separator();
	if (rsProject == r.state)
	{
		const bool remove = XUI::MenuItem(EditorUiContract::kCardDelete, XUI::Icon::Delete, nullptr, true, true);
		XUI::Tip("Deletes the project folder from disk for good. Asks first, naming what goes.");
		if (remove)
			OpenDeleteConfirm(r.path);
	}
	else
	{
		const bool forget = XUI::MenuItem(EditorUiContract::kCardRemoveFromList, XUI::Icon::Clear, nullptr, true);
		XUI::Tip(missing
			? "Takes the project off this list. Its folder is gone already."
			: "Takes the folder off this list. It holds no project.ltx or mod.ltx any more; nothing on disk changes.");
		if (forget)
			strncpy_s(s_ForgetPending, sizeof(s_ForgetPending), r.path, _TRUNCATE);
	}
	ImGui::EndPopup();
}

// A dialog of its own: the page under it is no modal any more, so nothing closes it
// by opening next to it. *open false (Cancel, Esc, MCP show=none) takes it down.
static void DrawDeleteConfirm()
{
	if (!XUI::BeginDialog(EditorUiContract::kDeleteProjectDialog, &s_DeleteWant, XUI::DialogSize::M))
		return;
	s_DeleteShown = true;

	const EditorProject::SDeleteInfo& d = s_DeleteDone ? s_DeleteResult : s_DeletePlan;
	char name[MAX_PATH * 3], path[MAX_PATH * 3], where[MAX_PATH * 3], line[MAX_PATH * 4];
	AcpToUtf8(d.name.c_str(), name, sizeof(name));
	AcpToUtf8(d.path.c_str(), path, sizeof(path));
	const xr_string held = CountText(d.files, "file", "files") + " in " + CountText(d.folders, "folder", "folders") + ", " + SizeText(d.bytes);
	const bool question = !s_DeleteDone && d.refused.empty() && !d.missing;
	const bool partial = s_DeleteDone && d.refused.empty();

	XUI::BeginDialogBody();
	if (question)		sprintf_s(line, "Delete the project \"%s\" from disk?", name);
	else if (partial)	sprintf_s(line, "The project \"%s\" was not deleted completely.", name);
	else if (d.missing)	sprintf_s(line, "The folder of the project \"%s\" does not exist any more.", name);
	else				sprintf_s(line, "The project \"%s\" cannot be deleted from here.", name);
	HeadlineText(line);
	ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::U32(CT::TextSecondary));
	XUI::TextEllipsis(path, -1.f, XUI::Clip::Start);
	ImGui::PopStyleColor();
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));
	if (question)
	{
		SecondaryText("What goes:");
		WhatGoes(held.c_str());
		if (d.git) WhatGoes(".git - the git repository, with the project's whole history");
		if (d.releases)
		{
			sprintf_s(line, "releases\\ - the files of published releases: %s, %s",
				CountText(d.release_files, "file", "files").c_str(), SizeText(d.release_bytes).c_str());
			WhatGoes(line);
		}
		if (d.links)
		{
			sprintf_s(line, "%s - removed as links; what they point at stays", CountText(d.links, "link", "links").c_str());
			WhatGoes(line);
		}
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));
		XUI::StatusText(XUI::Tone::Warning, "Deleted for good - this cannot be undone.");
	}
	else if (partial)
	{
		sprintf_s(line, "Deleted: %s. What stayed:", held.c_str());
		SecondaryText(line);
		const u32 listed = _min(u32(d.failed.size()), 10u);
		for (u32 i = 0; i < listed; ++i)
		{
			AcpToUtf8(d.failed[i].path.c_str(), where, sizeof(where));
			sprintf_s(line, "%s - %s", where, d.failed[i].reason.c_str());
			WhatGoes(line);
		}
		if (d.failed_total > listed)
		{
			sprintf_s(line, "and %u more", d.failed_total - listed);
			WhatGoes(line);
		}
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));
		XUI::StatusText(XUI::Tone::Info, "The project stays in the list. Close whatever holds these files, then delete it again.");
	}
	else if (d.missing)
		XUI::StatusText(XUI::Tone::Muted, "Nothing was deleted. Remove from List in its menu takes it off the list.");
	else
	{
		AcpToUtf8(d.refused.c_str(), line, sizeof(line));
		XUI::StatusText(XUI::Tone::Error, line);
		for (u32 i = 1; i < d.nested.size(); ++i)
		{
			AcpToUtf8(d.nested[i].c_str(), where, sizeof(where));
			WhatGoes(where);
		}
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));
		XUI::StatusText(XUI::Tone::Muted, "Nothing was deleted.");
	}

	// Cancel is the default; with Delete on the footer Enter and Esc both back out
	bool confirm = false, close = false;
	if (XUI::BeginDialogFooter())
	{
		if (question)
		{
			confirm = XUI::DialogButton("Delete", XUI::Kind::Danger);
			close = XUI::DialogButton("Cancel", XUI::Kind::Secondary, true);
		}
		else
			close = XUI::DialogButton("Close", XUI::Kind::Secondary, true);
	}
	if (close)
	{
		s_DeleteWant = false;
		ImGui::CloseCurrentPopup();
	}
	else if (confirm)
	{
		xr_string err;
		if (EditorProject::DeleteProject(s_DeletePlan.path.c_str(), s_DeleteResult, err))
		{
			s_DeleteWant = false;
			ImGui::CloseCurrentPopup();
			if (s_DeleteResult.missing)
				sprintf_s(line, "Project \"%s\" removed from the list: its folder was gone already.", name);
			else
				sprintf_s(line, "Project \"%s\" deleted: %s, %s.", name, CountText(s_DeleteResult.files, "file", "files").c_str(),
					CountText(s_DeleteResult.folders, "folder", "folders").c_str());
			XUI::Toast(XUI::Tone::Success, line);
		}
		else s_DeleteDone = true;
	}
	XUI::EndDialog();
}

//------------------------------------------------------------------------------
// MCP
//------------------------------------------------------------------------------
static bool ArgPath(LPCSTR raw, char* path, u32 size)
{
	if (!XFinedMCP::GetArg(raw, "path", path, size) || !path[0]) return false;
	for (char* p = path; *p; ++p) if ('/' == *p) *p = '\\';
	return true;
}

static bool ArgTrue(LPCSTR raw, LPCSTR field)
{
	char pat[64];
	sprintf_s(pat, "\"%s\"", field);
	LPCSTR k = raw ? strstr(raw, pat) : 0;
	if (!k) return false;
	k += xr_strlen(pat);
	while (' ' == *k || '\t' == *k) ++k;
	if (':' != *k) return false;
	++k;
	while (' ' == *k || '\t' == *k) ++k;
	return 0 == strncmp(k, "true", 4);
}

static void JsonError(xr_string& out, const xr_string& err)
{
	out = "{\"ok\":false,\"error\":\"";
	EditorGameContent::JsonAppendPath(out, err.c_str());
	out += "\"}";
}

// the fields a deletion answers with; `counts` names the object the numbers go
// into, or none for flat numbers
static void AppendDeleteJson(xr_string& out, const EditorProject::SDeleteInfo& d, LPCSTR counts)
{
	out += ",\"path\":\"";
	EditorGameContent::JsonAppendPath(out, d.path.c_str());
	out += "\",\"name\":\"";
	EditorGameContent::JsonAppendPath(out, d.name.c_str());
	out += d.refused.empty() ? "\",\"refused\":false" : "\",\"refused\":true";
	if (d.missing) out += ",\"missing\":true";
	char tmp[320];
	if (counts)
	{
		sprintf_s(tmp, ",\"%s\":{\"files\":%u,\"folders\":%u,\"links\":%u,\"bytes\":%llu}", counts, d.files, d.folders, d.links, d.bytes);
		out += tmp;
	}
	else
	{
		sprintf_s(tmp, ",\"files\":%u,\"folders\":%u,\"links\":%u,\"bytes\":%llu", d.files, d.folders, d.links, d.bytes);
		out += tmp;
	}
	sprintf_s(tmp, ",\"git\":%s,\"releases\":%s,\"release_files\":%u,\"release_bytes\":%llu,\"nested_total\":%u,\"failed_total\":%u",
		d.git ? "true" : "false", d.releases ? "true" : "false", d.release_files, d.release_bytes, d.nested_total, d.failed_total);
	out += tmp;
	out += ",\"nested\":[";
	for (u32 i = 0; i < d.nested.size(); ++i)
	{
		out += i ? ",\"" : "\"";
		EditorGameContent::JsonAppendPath(out, d.nested[i].c_str());
		out += "\"";
	}
	out += "],\"failed\":[";
	for (u32 i = 0; i < d.failed.size(); ++i)
	{
		out += i ? ",{\"path\":\"" : "{\"path\":\"";
		EditorGameContent::JsonAppendPath(out, d.failed[i].path.c_str());
		out += "\",\"reason\":\"";
		EditorGameContent::JsonAppendPath(out, d.failed[i].reason.c_str());
		out += "\"}";
	}
	out += "]";
}

void EditorProject::McpReveal(LPCSTR raw, xr_string& out)
{
	char path[MAX_PATH * 2] = {};
	if (!ArgPath(raw, path, sizeof(path)))
	{
		JsonError(out, "missing 'path' - a project folder from list_projects");
		return;
	}
	xr_string folder, err;
	if (!RevealTarget(path, folder, err))
	{
		JsonError(out, err);
		return;
	}
	// dry_run answers the folder and opens nothing: a test leaves no window behind
	const bool dry = ArgTrue(raw, "dry_run");
	if (!dry && !RevealProject(folder.c_str(), err))
	{
		JsonError(out, err);
		return;
	}
	out = "{\"ok\":true,\"path\":\"";
	EditorGameContent::JsonAppendPath(out, folder.c_str());
	out += dry ? "\",\"opened\":false,\"dry_run\":true}" : "\",\"opened\":true,\"dry_run\":false}";
}

void EditorProject::McpDelete(LPCSTR raw, xr_string& out)
{
	char path[MAX_PATH * 2] = {};
	if (!ArgPath(raw, path, sizeof(path)))
	{
		JsonError(out, "missing 'path' - a project folder from list_projects");
		return;
	}
	// the caller decided: no confirmation on this path, the same checks as the window's
	SDeleteInfo info;
	xr_string err;
	const bool ok = DeleteProject(path, info, err);
	out = ok ? "{\"ok\":true" : "{\"ok\":false,\"error\":\"";
	if (!ok)
	{
		EditorGameContent::JsonAppendPath(out, err.c_str());
		out += "\"";
	}
	if (ok && info.missing) out += ",\"note\":\"the folder does not exist any more - removed from the list only\"";
	AppendDeleteJson(out, info, "deleted");
	out += info.removed_from_list ? ",\"removed_from_list\":true}" : ",\"removed_from_list\":false}";
}

void EditorProject::McpForget(LPCSTR raw, xr_string& out)
{
	char path[MAX_PATH * 2] = {};
	if (!ArgPath(raw, path, sizeof(path)))
	{
		JsonError(out, "missing 'path' - a folder from list_projects");
		return;
	}
	xr_string err;
	if (!ForgetProject(path, err))
	{
		JsonError(out, err);
		return;
	}
	out = "{\"ok\":true,\"path\":\"";
	EditorGameContent::JsonAppendPath(out, path);
	out += "\",\"removed_from_list\":true}";
}

static SRecentProject* FindTile(LPCSTR path)
{
	for (int i = 0; i < s_RecentCount; ++i)
		if (SamePath(s_Recent[i].path, path)) return &s_Recent[i];
	return 0;
}

// the menu a card offers in its state, as the page draws it
static void AppendMenuItems(xr_string& out, ERecentState state)
{
	out += ",\"items\":[{\"label\":\"";
	out += EditorUiContract::kCardShowInExplorer;
	out += rsMissing == state ? "\",\"enabled\":false}" : "\",\"enabled\":true}";
	out += ",{\"label\":\"";
	out += rsProject == state ? EditorUiContract::kCardDelete : EditorUiContract::kCardRemoveFromList;
	out += "\",\"enabled\":true}]";
}

void EditorProject::McpBrowserMenu(LPCSTR raw, xr_string& out)
{
	char show[16] = {}, path[MAX_PATH * 2] = {}, search[sizeof(s_Search)] = {}, sort[16] = {};
	XFinedMCP::GetArg(raw, "show", show, sizeof(show));
	ArgPath(raw, path, sizeof(path));
	const bool set_search = XFinedMCP::GetArg(raw, "search", search, sizeof(search));
	const bool set_sort = XFinedMCP::GetArg(raw, "sort", sort, sizeof(sort)) && sort[0];
	if ((show[0] || set_search || set_sort) && s_Active)
	{
		JsonError(out, "the project browser is not on screen: a project is open");
		return;
	}

	// everything is checked before anything changes
	int sort_index = s_Sort;
	if (set_sort)
	{
		sort_index = -1;
		for (int i = 0; i < 2; ++i)
			if (0 == xr_strcmp(sort, kSortKeys[i])) sort_index = i;
		if (sort_index < 0)
		{
			JsonError(out, "sort: recent or name");
			return;
		}
	}
	const bool none = 0 == xr_strcmp(show, "none"), menu = 0 == xr_strcmp(show, "menu"), confirm = 0 == xr_strcmp(show, "confirm");
	SRecentProject* target = 0;
	if (show[0] && !none)
	{
		if (!menu && !confirm)
		{
			JsonError(out, "show: menu, confirm or none");
			return;
		}
		target = path[0] ? FindTile(path) : 0;
		if (!target)
		{
			JsonError(out, xr_string("no card of the project browser shows '") + path + "'");
			return;
		}
		target->state = RecentState(target->path);
		if (confirm && rsProject != target->state)
		{
			JsonError(out, "that folder holds no project: its menu offers Remove from List, which asks nothing");
			return;
		}
	}

	// what the page shows changes; nothing is saved and nothing is acted on
	s_Sort = sort_index;
	if (set_search) strncpy_s(s_Search, sizeof(s_Search), search, _TRUNCATE);
	if (none)
	{
		// both go down, and nothing is deleted
		s_MenuFor[0]	= 0;
		s_MenuClose		= true;
		s_DeleteWant	= false;
	}
	else if (target)
	{
		// a card the search hides has no menu to show: the search goes
		if (!XUI::Match(s_Search, (target->shown_name + " " + target->shown_path).c_str())) s_Search[0] = 0;
		if (menu)
		{
			s_DeleteWant	= false;
			s_MenuClose		= false;
			strncpy_s(s_MenuFor, sizeof(s_MenuFor), target->path, _TRUNCATE);
		}
		else
		{
			s_MenuFor[0]	= 0;
			s_MenuClose		= true;
			// the window's own opening: the check and the count, then the question
			OpenDeleteConfirm(target->path);
		}
	}

	out = s_Active ? "{\"ok\":true,\"browser\":false" : "{\"ok\":true,\"browser\":true";
	// the menu: asked for and not drawn yet, or what the last frame drew
	LPCSTR shown_menu = s_MenuFor[0] ? s_MenuFor : (!s_MenuClose && s_MenuShown[0]) ? s_MenuShown : 0;
	SRecentProject* tile = shown_menu ? FindTile(shown_menu) : 0;
	if (tile)
	{
		out += ",\"menu\":{\"path\":\"";
		EditorGameContent::JsonAppendPath(out, tile->path);
		out += "\",\"shown\":";
		out += s_MenuFor[0] ? "false" : "true";
		out += ",\"state\":\"";
		out += kRecentStateNames[tile->state];
		out += "\"";
		AppendMenuItems(out, tile->state);
		out += "}";
	}
	else out += ",\"menu\":null";
	if (s_DeleteWant)
	{
		const SDeleteInfo& d = s_DeleteDone ? s_DeleteResult : s_DeletePlan;
		// the dialog's id, what screenshot_editor names as modal.id while it is up
		out += ",\"confirm\":{\"id\":\"";
		out += DialogName(EditorUiContract::kDeleteProjectDialog);
		out += "\",\"shown\":";
		out += s_DeleteShown ? "true" : "false";
		out += ",\"view\":\"";
		out += s_DeleteDone ? "result" : (!d.refused.empty() || d.missing) ? "refused" : "question";
		out += "\",\"reason\":\"";
		EditorGameContent::JsonAppendPath(out, d.refused.c_str());
		out += "\"";
		AppendDeleteJson(out, d, 0);
		out += "}";
	}
	else out += ",\"confirm\":null";

	// the cards in the order the page draws them
	out += ",\"search\":";
	NqUtil::JsonString(out, s_Search);
	out += ",\"sort\":\"";
	out += kSortKeys[s_Sort];
	out += "\",\"cards\":[";
	xr_vector<int> order;
	if (!s_Active) CardOrder(order);
	for (u32 i = 0; i < order.size(); ++i)
	{
		const SRecentProject& r = s_Recent[order[i]];
		out += i ? ",{\"path\":\"" : "{\"path\":\"";
		EditorGameContent::JsonAppendPath(out, r.path);
		out += "\",\"name\":";
		NqUtil::JsonString(out, r.shown_name);
		out += ",\"state\":\"";
		out += kRecentStateNames[r.state];
		out += "\",\"parent\":";
		NqUtil::JsonString(out, r.shown_parent);
		// what the card's picture area holds: preview.png, or the placeholder for none or one of no scene
		out += ",\"preview\":\"";
		out += rsProject != r.state ? "placeholder" : !r.preview_tried ? "not_drawn" : r.preview ? "picture" : "placeholder";
		out += "\"}";
	}
	out += "]}";
}

void EditorProject::McpCreate(LPCSTR raw, xr_string& out)
{
	char parent[MAX_PATH * 2] = {}, name[64] = {};
	XFinedMCP::GetArg(raw, "name", name, sizeof(name));
	// no parent: the location the Start page's form shows
	if (!XFinedMCP::GetArg(raw, "parent", parent, sizeof(parent)) || !parent[0])
		strcpy_s(parent, s_NewParent);
	for (char* p = parent; *p; ++p) if ('/' == *p) *p = '\\';
	while (xr_strlen(parent) > 3 && '\\' == parent[xr_strlen(parent) - 1]) parent[xr_strlen(parent) - 1] = 0;
	if (s_Active)
	{
		JsonError(out, "a project is open: project_close first, then create");
		return;
	}
	if (xr_strlen(parent) + xr_strlen(name) + 2 > MAX_PATH)
	{
		JsonError(out, "the path is too long");
		return;
	}
	char folder[MAX_PATH];
	sprintf_s(folder, "%s\\%s", parent, name);
	const bool existed = DirExists(folder) && HasManifest(folder);
	if (!Create(parent, name))
	{
		JsonError(out, s_CreateError[0] ? s_CreateError : "the project could not be created");
		return;
	}
	out = "{\"ok\":true,\"path\":\"";
	EditorGameContent::JsonAppendPath(out, s_Project);
	out += "\",\"name\":";
	NqUtil::JsonString(out, name);
	out += existed ? ",\"existed\":true,\"opened\":true}" : ",\"existed\":false,\"opened\":true}";
}

//------------------------------------------------------------------------------
// the Start page (the whole work area, drawn instead of the editor UI)
//------------------------------------------------------------------------------
// -project <folder|name>: opens that project at startup and skips the picker
// entirely, so a scripted run - a smoke test, an agent driving the MCP port -
// lands in the right project without anyone clicking a tile. The argument is
// either a project folder or the name of a recent one; quotes are honoured
// because a path with spaces is the normal case.
static bool OpenProjectFromCommandLine()
{
	LPCSTR cmd = ::GetCommandLineA();
	LPCSTR at  = strstr(cmd, "-project");
	if (!at)	return false;

	LPCSTR p = at + xr_strlen("-project");
	while (' ' == *p || '=' == *p || ':' == *p) ++p;
	if (!*p)	return false;

	string_path arg = {};
	if ('"' == *p)
	{
		++p;
		LPCSTR end = strchr(p, '"');
		const int len = end ? int(end - p) : int(xr_strlen(p));
		strncpy_s(arg, sizeof(arg), p, _TRUNCATE);
		if (len < (int)sizeof(arg)) arg[len] = 0;
	}
	else
	{
		LPCSTR end = strchr(p, ' ');
		const int len = end ? int(end - p) : int(xr_strlen(p));
		strncpy_s(arg, sizeof(arg), p, _TRUNCATE);
		if (len < (int)sizeof(arg)) arg[len] = 0;
	}
	if (!arg[0])	return false;

	for (char* c = arg; *c; ++c) if ('/' == *c) *c = '\\';

	if (EditorProject::Open(arg))
	{
		Msg("* -project: opened '%s'", arg);
		return true;
	}

	// not a folder - try it as the NAME of a project we already know
	for (int i = 0; i < s_RecentCount; ++i)
		if (0 == _stricmp(s_Recent[i].name, arg) && EditorProject::Open(s_Recent[i].path))
		{
			Msg("* -project: opened recent '%s'", s_Recent[i].path);
			return true;
		}

	Msg("! -project: cannot open '%s' - falling back to the project browser", arg);
	return false;
}

// Runs from the frame loop, not from the browser's Draw: a window that starts
// minimized or hidden draws no UI at all, and a scripted run - the MCP port
// answers from the first idle tick - would wait for a project forever.
void EditorProject::Startup()
{
	static bool s_loaded = false;
	if (s_loaded) return;
	s_loaded = true;
	ResolveAppRoot(); LoadRecent();
	// after the recent list is up: matching by name needs it
	OpenProjectFromCommandLine();
}

// The create form's look at its target, taken again only when a field changes:
// every keystroke would otherwise be a round of file operations
static void CheckNewFolder()
{
	char key[sizeof(s_NewChecked)];
	_snprintf_s(key, _TRUNCATE, "%s|%s", s_NewParent, s_NewName);
	if (0 == strcmp(key, s_NewChecked)) return;
	strcpy_s(s_NewChecked, key);
	// the fields changed: the last failure spoke of something else
	s_CreateError[0] = 0;
	s_NewState = nsNew;
	if (s_NewParent[0] && !IsAbsolutePath(s_NewParent))
	{
		s_NewState = nsNotFullPath;
		return;
	}
	if (!s_NewParent[0] || !EditorProject::ValidateName(s_NewName) || xr_strlen(s_NewParent) + xr_strlen(s_NewName) + 2 > MAX_PATH)
		return;
	char folder[MAX_PATH];
	sprintf_s(folder, "%s\\%s", s_NewParent, s_NewName);
	if (DirExists(folder)) s_NewState = HasManifest(folder) ? nsProjectThere : nsFolderThere;
}

static void FormMessage(XUI::Tone tone, LPCSTR text)
{
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
	XUI::StatusText(tone, text);
}

// Left of the cards: New Project (Location, Name, Create) and Open Folder...
static void DrawNewProject(bool& create, bool& open_folder)
{
	const float width = _min(ImGui::GetContentRegionAvail().x, XUI::Pt(kFormW));
	{
		XUI::FontScope title(XUI::Font::Title);
		ImGui::TextUnformatted("New Project");
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
	SecondaryText("Location");
	ImGui::SetNextItemWidth(width);
	PathFieldAcp("##location", s_NewParent, sizeof(s_NewParent));
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
	SecondaryText("Name");
	ImGui::SetNextItemWidth(width);
	const bool enter = ImGui::InputTextWithHint("##name", "Latin letters, digits, _ and -", s_NewName, sizeof(s_NewName),
		ImGuiInputTextFlags_EnterReturnsTrue);
	CheckNewFolder();

	// the worst first
	const bool name_ok = EditorProject::ValidateName(s_NewName);
	if (s_NewName[0] && !name_ok)				FormMessage(XUI::Tone::Error, "Use Latin letters, digits, _ and - only.");
	else if (nsNotFullPath == s_NewState)		FormMessage(XUI::Tone::Error, "The location must be a full path.");
	else if (nsProjectThere == s_NewState)		FormMessage(XUI::Tone::Info, "A project with this name is there already: Create opens it.");
	else if (nsFolderThere == s_NewState)		FormMessage(XUI::Tone::Warning, "The folder exists: the project is made inside it.");
	if (s_CreateError[0])
	{
		char text[sizeof(s_CreateError) * 2];
		AcpToUtf8(s_CreateError, text, sizeof(text));
		FormMessage(XUI::Tone::Error, text);
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));

	LPCSTR why = !s_NewParent[0] ? "Pick a location first." : nsNotFullPath == s_NewState ? "The location must be a full path."
		: !s_NewName[0] ? "Enter a name first." : !name_ok ? "The name may hold Latin letters, digits, _ and - only." : 0;
	if (XUI::Button("Create", XUI::Kind::Primary, !why, why) || (enter && !why))
		create = true;

	ImGui::Dummy(ImVec2(0.f, XUI::Pt(16.f)));
	const ImVec2 at = ImGui::GetCursorScreenPos();
	ImGui::GetWindowDrawList()->AddLine(ImVec2(at.x, Snap(at.y)), ImVec2(at.x + width, Snap(at.y)), XFinedTheme::U32(CT::BorderSubtle),
		_max(1.f, XUI::Pt(1.f)));
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(16.f)));
	if (XUI::Button("Open Folder..."))
		open_folder = true;
	XUI::Tip("Opens a folder that holds mod.ltx or project.ltx, such as a fresh clone of a mod's repository.");
}

static void OpenFolder()
{
	char picked[MAX_PATH] = {};
	if (!PickFolder(picked, sizeof(picked), L"Open Project Folder")) return;
	char shown[MAX_PATH * 3];
	AcpToUtf8(picked, shown, sizeof(shown));
	if (!HasManifest(picked))
		s_PageError = xr_string("Not a project folder - it holds no mod.ltx or project.ltx: ") + shown;
	else if (!EditorProject::Open(picked))
		s_PageError = xr_string("The project cannot be opened: ") + EditorProject::OpenError();
}

// The card's path in the font the caller pushed, cut at the start. For a twin the folder that tells it apart
// stays whole and reads in the text colour: when the path from it on is too long, the cut moves between it
// and the leaf too ("...\tour3a\...\Name"); only a folder wider than the card is cut itself. A cut path is
// the tooltip.
static void DrawCardPath(const SRecentProject& r, float room)
{
	ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::U32(CT::TextSecondary));
	if (r.parent_end <= r.parent_begin)
	{
		XUI::TextEllipsis(r.shown_path.c_str(), room, XUI::Clip::Start);
		ImGui::PopStyleColor();
		return;
	}
	LPCSTR path = r.shown_path.c_str(), seg = path + r.parent_begin, seg_end = path + r.parent_end;
	// a cut before the folder only where something stands before it, after it only where folders stand
	// between it and the leaf
	static const char kCut[] = "...\\";
	LPCSTR lead = r.parent_begin ? kCut : path, lead_end = r.parent_begin ? kCut + sizeof(kCut) - 1 : path;
	const xr_string past = r.leaf_begin > r.parent_end + 1 ? xr_string("\\...\\") + (path + r.leaf_begin) : xr_string(seg_end);
	// what stands before and after the folder, tried in this order: the whole path, cut before the folder,
	// cut before and after it
	struct STry { LPCSTR pre, pre_end, post; };
	const STry tries[] =
	{
		{ path, seg, seg_end },
		{ lead, lead_end, seg_end },
		{ lead, lead_end, past.c_str() },
	};
	const float seg_w = ImGui::CalcTextSize(seg, seg_end).x;
	const STry* pick = 0;
	for (const STry& t : tries)
		if (ImGui::CalcTextSize(t.pre, t.pre_end).x + seg_w + ImGui::CalcTextSize(t.post).x <= room)
		{
			pick = &t;
			break;
		}
	ImGui::BeginGroup();
	if (pick)
	{
		ImGui::TextUnformatted(pick->pre, pick->pre_end);
		ImGui::SameLine(0.f, 0.f);
		ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::U32(CT::Text));
		ImGui::TextUnformatted(seg, seg_end);
		ImGui::PopStyleColor();
		ImGui::SameLine(0.f, 0.f);
		ImGui::TextUnformatted(pick->post);
	}
	else
	{
		// plain text: the one tooltip is the whole path's, below
		xr_string shown;
		const bool cut = XUI::EllipsizeText(r.shown_parent.c_str(), room, XUI::Clip::End, shown);
		ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::U32(CT::Text));
		ImGui::TextUnformatted(cut ? shown.c_str() : r.shown_parent.c_str());
		ImGui::PopStyleColor();
	}
	ImGui::EndGroup();
	if (pick != tries) XUI::Tip(r.shown_path.c_str());
	ImGui::PopStyleColor();
}

// One card, 232 x 184 pt: the preview, the name and the ... menu, the path with a start
// ellipsis (a twin's differing parent kept whole in it). True on a click that opens it.
static bool DrawCard(SRecentProject& r, const ImVec2& at)
{
	const float w = XUI::Pt(kCardW), h = XUI::Pt(kCardH), pad = XUI::Pt(8.f);
	const bool here = rsProject == r.state;
	if (here && !r.preview_tried)
	{
		r.preview = LoadPreview(r.path, r.preview_w, r.preview_h);
		r.preview_tried = true;
	}
	// by folder, not by slot: a card leaving the list must not hand its menu on
	ImGui::PushID(r.path);
	ImGui::SetCursorScreenPos(at);
	ImGui::SetNextItemAllowOverlap();
	const bool clicked = ImGui::InvisibleButton("##card", ImVec2(w, h));
	const bool right_click = ImGui::IsMouseReleased(ImGuiMouseButton_Right) && ImGui::IsItemHovered(ImGuiHoveredFlags_AllowWhenBlockedByPopup);
	if (!here) XUI::Tip(StateHelp(r.state));
	const ImVec2 min = at, max(at.x + w, at.y + h);
	const bool hovered = ImGui::IsMouseHoveringRect(min, max) && ImGui::IsWindowHovered();
	// a folder that holds no project any more is veiled and opens nothing
	const float veil = here ? 1.f : .5f;

	ImDrawList* dl = ImGui::GetWindowDrawList();
	const float radius = XUI::Pt(6.f);
	dl->AddRectFilled(min, max, XFinedTheme::U32(hovered ? CT::Hover : CT::Raised), radius);
	dl->AddRect(min, max, XFinedTheme::U32(hovered ? CT::Border : CT::BorderSubtle), radius, 0, _max(1.f, XUI::Pt(1.f)));

	// the preview is cut to the card's shape, never stretched
	const ImVec2 pmin(min.x + pad, min.y + pad), pmax(max.x - pad, min.y + pad + XUI::Pt(kPreviewH));
	if (r.preview && r.preview_w && r.preview_h)
	{
		const float want = (pmax.x - pmin.x) / (pmax.y - pmin.y), have = float(r.preview_w) / float(r.preview_h);
		ImVec2 uv0(0.f, 0.f), uv1(1.f, 1.f);
		if (have > want)	{ uv0.x = (1.f - want / have) * .5f; uv1.x = 1.f - uv0.x; }
		else				{ uv0.y = (1.f - have / want) * .5f; uv1.y = 1.f - uv0.y; }
		dl->AddImageRounded(r.preview, pmin, pmax, uv0, uv1, ImGui::GetColorU32(IM_COL32_WHITE, veil), XUI::Pt(4.f));
	}
	else
	{
		dl->AddRectFilled(pmin, pmax, XFinedTheme::U32(CT::Canvas, veil), XUI::Pt(4.f));
		const float icon = XUI::Pt(32.f);
		XUI::DrawIcon(dl, here ? XUI::Icon::Level : XUI::Icon::Folder, ImVec2(Snap((pmin.x + pmax.x - icon) * .5f),
			Snap((pmin.y + pmax.y - icon) * .5f)), 32.f, XFinedTheme::U32(CT::TextMuted, veil));
	}
	if (!here)
	{
		ImGui::SetCursorScreenPos(ImVec2(pmin.x + XUI::Pt(6.f), pmin.y + XUI::Pt(6.f)));
		XUI::Badge(rsMissing == r.state ? "Missing" : "Not a project", XUI::Tone::Warning);
	}

	// the name has its row to itself, the ... menu at its end; the path under it, holding a twin's parent
	const float row_y = pmax.y + XUI::Pt(6.f), row_h = XUI::Pt(24.f), more = XUI::Pt(24.f);
	const float name_room = w - pad * 2.f - more - XUI::Pt(4.f);
	ImGui::PushStyleVar(ImGuiStyleVar_Alpha, ImGui::GetStyle().Alpha * veil);
	{
		XUI::FontScope strong(XUI::Font::Strong);
		ImGui::SetCursorScreenPos(ImVec2(pmin.x, Snap(row_y + (row_h - ImGui::GetTextLineHeight()) * .5f)));
		XUI::TextEllipsis(r.shown_name.c_str(), name_room);
	}
	{
		XUI::FontScope small_font(XUI::Font::Small);
		ImGui::SetCursorScreenPos(ImVec2(pmin.x, row_y + row_h + XUI::Pt(2.f)));
		DrawCardPath(r, w - pad * 2.f);
	}
	ImGui::PopStyleVar();

	ImGui::SetCursorScreenPos(ImVec2(max.x - pad - more, row_y));
	const bool more_clicked = XUI::IconButton("##more", XUI::Icon::More, "Actions");
	const ImVec2 below(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y + XUI::Pt(2.f));
	if (more_clicked)
		XUI::Shell::OpenPopupAt("##card_menu", below);
	else if (right_click)
		XUI::Shell::OpenPopupAt("##card_menu", ImGui::GetMousePos());
	// MCP asks the way the ... button does; a card scrolled away comes into view first
	if (s_MenuFor[0] && 0 == _stricmp(s_MenuFor, r.path))
	{
		if (ImGui::IsRectVisible(min, max))
		{
			s_MenuFor[0] = 0;
			XUI::Shell::OpenPopupAt("##card_menu", below);
		}
		else
			ImGui::SetScrollFromPosY(min.y - ImGui::GetWindowPos().y, .3f);
	}
	DrawCardMenu(r);
	ImGui::PopID();
	return clicked && here;
}

// The cards in rows as wide as the room allows; the index of the one clicked, or -1
static int DrawCardGrid(const xr_vector<int>& order)
{
	const float w = XUI::Pt(kCardW), h = XUI::Pt(kCardH), gap = XUI::Pt(kCardGap);
	const float avail = ImGui::GetContentRegionAvail().x;
	const int per_row = _max(1, int((avail + gap) / (w + gap)));
	const ImVec2 origin = ImGui::GetCursorScreenPos();
	int open = -1;
	for (u32 n = 0; n < order.size(); ++n)
	{
		const ImVec2 at(origin.x + float(n % per_row) * (w + gap), origin.y + float(n / per_row) * (h + gap));
		if (DrawCard(s_Recent[order[n]], at)) open = order[n];
	}
	// the grid as one block, so the window knows how far it reaches
	const int rows = (int(order.size()) + per_row - 1) / per_row;
	ImGui::SetCursorScreenPos(origin);
	ImGui::Dummy(ImVec2(_min(avail, float(per_row) * (w + gap) - gap), rows ? float(rows) * (h + gap) - gap : 0.f));
	return open;
}

// Right of the form: search and sort, then the cards on an inset panel of their own
static int DrawProjects(bool own_scroll)
{
	// the sort as wide as its segments and a point more (one pixel short, and it turns into a combo); the search
	// takes the rest of the row
	float segment = 0.f;
	for (LPCSTR label : kSortLabels) segment = _max(segment, XUI::TextWidth(XUI::Font::Body, label) + XUI::Pt(20.f));
	const float sort_w = segment * 2.f + XUI::Pt(5.f);
	ImGui::SetNextItemWidth(_max(XUI::Em(8.f), ImGui::GetContentRegionAvail().x - sort_w - XUI::Pt(8.f)));
	XUI::SearchField("##search", s_Search, sizeof(s_Search), "Search projects");
	ImGui::SameLine(0.f, XUI::Pt(8.f));
	int sort = s_Sort;
	if (XUI::Segmented("##sort", &sort, kSortLabels, 2))
	{
		s_Sort = sort;
		char ini[MAX_PATH];
		RecentIni(ini, sizeof(ini));
		::WritePrivateProfileStringA("browser", "sort", kSortKeys[s_Sort], ini);
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));

	xr_vector<int> order;
	CardOrder(order);
	if (own_scroll)
	{
		ImGui::PushStyleColor(ImGuiCol_ChildBg, XFinedTheme::U32(CT::Inset));
		ImGui::PushStyleVar(ImGuiStyleVar_ChildRounding, XUI::Pt(6.f));
		ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(16.f), XUI::Pt(16.f)));
		ImGui::BeginChild("##cards", ImVec2(0.f, 0.f), ImGuiChildFlags_AlwaysUseWindowPadding);
		ImGui::PopStyleVar(2);
		ImGui::PopStyleColor();
	}
	int open = -1;
	if (!s_RecentCount)
		XUI::EmptyState(XUI::Icon::Folder, "No projects yet", "Create one with New Project, or open a folder that holds one.");
	else if (order.empty())
		XUI::EmptyState(XUI::Icon::Search, "No matching projects", "No project name or folder holds every word of the search.");
	else
		open = DrawCardGrid(order);
	if (own_scroll) ImGui::EndChild();
	return open;
}

bool EditorProject::DrawBrowser()
{
	// the frame that drew them is out by now
	ReleaseDeadPreviews();
	Startup();
	if (s_Active)
	{
		// a project opened under the confirmation (over MCP): the confirmation goes with the page
		if (s_DeleteWant || s_DeleteShown) XUI::CloseDialog(DialogName(EditorUiContract::kDeleteProjectDialog));
		s_DeleteWant = s_DeleteShown = false;
		s_MenuFor[0] = s_MenuShown[0] = 0;
		return false;
	}
	// what this frame draws of the menu and the confirmation, for McpBrowserMenu
	s_MenuShown[0]	= 0;
	s_DeleteShown	= false;

	int open = -1;
	bool create = false, open_folder = false;
	if (XUI::BeginPage(EditorUiContract::kWindowStartPage))
	{
		{
			XUI::FontScope page(XUI::Font::Page);
			ImGui::TextUnformatted("Projects");
		}
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));
		if (!s_PageError.empty())
		{
			if (XUI::Banner(XUI::Tone::Error, s_PageError.c_str(), "Dismiss")) s_PageError.clear();
			ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));
		}
		const ImVec2 avail = ImGui::GetContentRegionAvail();
		// narrower than the form beside one card: one column, and the page scrolls as a whole
		if (avail.x < XUI::Pt(kFormW + 24.f + kCardW + 32.f))
		{
			DrawNewProject(create, open_folder);
			ImGui::Dummy(ImVec2(0.f, XUI::Pt(24.f)));
			open = DrawProjects(false);
		}
		else
		{
			ImGui::BeginChild("##new", ImVec2(XUI::Pt(kFormW), avail.y));
			DrawNewProject(create, open_folder);
			ImGui::EndChild();
			ImGui::SameLine(0.f, XUI::Pt(24.f));
			ImGui::BeginChild("##projects", ImVec2(0.f, avail.y));
			open = DrawProjects(true);
			ImGui::EndChild();
		}
		DrawDeleteConfirm();
	}
	XUI::EndPage();

	// what the page asked for, now that it is drawn: the list is what the cards walk
	string_path open_path = {};
	if (open >= 0) xr_strcpy(open_path, s_Recent[open].path);
	if (s_ForgetPending[0])
	{
		if (SRecentProject* gone = FindTile(s_ForgetPending))
		{
			const xr_string toast = gone->shown_name + " was taken off the list.";
			ForgetRecent(s_ForgetPending);
			XUI::Toast(XUI::Tone::Info, toast.c_str());
		}
		s_ForgetPending[0] = 0;
	}
	s_MenuClose = false;
	if (create)
	{
		if (Create(s_NewParent, s_NewName))
		{
			s_NewName[0] = 0;
			s_NewChecked[0] = 0;
		}
	}
	else if (open_folder)
		OpenFolder();
	else if (open_path[0] && !Open(open_path))
	{
		// gone since the list was read, or refused
		if (SRecentProject* card = FindTile(open_path)) card->state = RecentState(card->path);
		s_PageError = xr_string("The project cannot be opened: ") + OpenError();
	}
	return true;
}

//------------------------------------------------------------------------------
// Link Game: a card on the Start page, drawn instead of the editor UI while the
// open project has no valid game. The only ways on are a valid link and Close
// Project, which goes back to the project list.
//------------------------------------------------------------------------------
bool EditorProject::DrawGameLink()
{
	if (!s_Active)		return false;
	if (GameLinked())	return false;

	// the hint follows the text, looked at again only when the text changed
	if (0 != strcmp(s_LinkPath, s_LinkChecked))
	{
		strcpy_s(s_LinkChecked, s_LinkPath);
		xr_string reason;
		s_LinkHintOk = EditorGameContent::LooksLikeGame(s_LinkPath, reason);
		if (s_LinkHintOk)	strcpy_s(s_LinkHint, "A Dead Air install: fsgame.ltx and its data are there.");
		else				strncpy_s(s_LinkHint, sizeof(s_LinkHint), reason.c_str(), _TRUNCATE);
	}
	char hint[sizeof(s_LinkHint) * 2], project[MAX_PATH * 3 + 16], text[MAX_PATH * 3];
	AcpToUtf8(s_LinkHint, hint, sizeof(hint));
	AcpToUtf8(s_Name, text, sizeof(text));
	sprintf_s(project, "Project: %s", text);

	int clicked = -1;
	if (XUI::BeginPage(EditorUiContract::kWindowStartPage))
	{
		XUI::BeginPageCard("##link_game", "Link Game", 560.f);
		XUI::Prose("Pick the Dead Air Refined install this project builds for: the folder that holds fsgame.ltx and database\\ "
			"or gamedata\\. The editor only reads it and never writes into it.");
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(12.f)));
		SecondaryText("Game folder");
		PathFieldAcp("##game_root", s_LinkPath, sizeof(s_LinkPath));
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
		if (!s_LinkPath[0])	XUI::StatusText(XUI::Tone::Muted, "No folder selected.");
		else				XUI::StatusText(s_LinkHintOk ? XUI::Tone::Success : XUI::Tone::Error, hint);
		if (s_LinkError[0])
		{
			AcpToUtf8(s_LinkError, text, sizeof(text));
			XUI::StatusText(XUI::Tone::Error, (xr_string("Link failed: ") + text).c_str());
		}
		// a link that went away mid-session leaves a scene open behind this card: it is not closed from here,
		// where nothing could ask about its unsaved changes
		const bool scene = HasLiveScene();
		const XUI::CardAction actions[] =
		{
			{ "Close Project",	XUI::Kind::Secondary,	false,	!scene,
				"A scene is open. Link the game to get back to it; File > Project > Switch Project... closes the project." },
			{ "Link",			XUI::Kind::Primary,		true,	s_LinkHintOk,	s_LinkPath[0] ? hint : "Pick the game folder first." },
		};
		clicked = XUI::CardFooter(project, XUI::Tone::Muted, actions, 2);
		XUI::EndPageCard();
	}
	XUI::EndPage();

	if (1 == clicked && LinkGame(s_LinkPath))
		s_LinkChecked[0] = 0;
	else if (0 == clicked)
	{
		// the project's tabs come back from its session when it opens again
		XUI::Shell::CloseProjectTabs();
		Close();
	}
	return true;
}

//------------------------------------------------------------------------------
// Import Base Scene: a dialog with a search; the library is read once, when it opens
//------------------------------------------------------------------------------
static bool CopyTree(const char* src, const char* dst)
{
	::CreateDirectoryA(dst, NULL);
	char mask[MAX_PATH];
	sprintf_s(mask, "%s\\*", src);
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (h == INVALID_HANDLE_VALUE) return false;
	do
	{
		if (!strcmp(fd.cFileName, ".") || !strcmp(fd.cFileName, "..")) continue;
		char a[MAX_PATH], b[MAX_PATH];
		sprintf_s(a, "%s\\%s", src, fd.cFileName);
		sprintf_s(b, "%s\\%s", dst, fd.cFileName);
		if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)	CopyTree(a, b);
		else												::CopyFileA(a, b, FALSE);
	} while (::FindNextFileA(h, &fd));
	::FindClose(h);
	return true;
}

void EditorProject::DrawUI()
{
	if (!s_WantImport && !s_ImportListed) return;
	// the library and what the project holds of it, read once as the dialog opens
	if (s_WantImport && !s_ImportListed)
	{
		s_ImportScenes.clear();
		s_ImportInProject.clear();
		if (s_Active)
		{
			ListBaseScenes(s_ImportScenes);
			for (const xr_string& scene : s_ImportScenes)
			{
				string_path file;
				sprintf_s(file, "%s\\rawdata\\levels\\%s.level", s_Project, scene.c_str());
				s_ImportInProject.push_back(INVALID_FILE_ATTRIBUTES != ::GetFileAttributesA(file) ? 1 : 0);
			}
		}
		s_ImportSearch[0] = 0;
		s_ImportPick.clear();
		s_ImportListed = true;
	}
	if (!XUI::BeginDialog(EditorUiContract::kDialogImportBaseScene, &s_WantImport, XUI::DialogSize::M))
	{
		// closed: the next opening reads the library again
		if (!s_WantImport) s_ImportListed = false;
		return;
	}

	XUI::BeginDialogBody();
	XUI::SearchField("##search", s_ImportSearch, sizeof(s_ImportSearch), "Search scenes");
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
	bool import = false;
	u32 shown = 0;
	const float badge = XUI::TextWidth(XUI::Font::Small, "In project") + XUI::Pt(12.f);
	if (XUI::BeginList("##scenes", ImVec2(0.f, 320.f)))
	{
		char name[MAX_PATH * 3];
		for (u32 i = 0; i < s_ImportScenes.size(); ++i)
		{
			const xr_string& scene = s_ImportScenes[i];
			AcpToUtf8(scene.c_str(), name, sizeof(name));
			if (!XUI::Match(s_ImportSearch, name)) continue;
			++shown;
			if (XUI::ListRow(scene.c_str(), scene == s_ImportPick)) s_ImportPick = scene;
			const ImVec2 row_min = ImGui::GetItemRectMin(), row_max = ImGui::GetItemRectMax();
			// a double click imports at once
			if (ImGui::IsItemHovered() && ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left))
			{
				s_ImportPick = scene;
				import = true;
			}
			const float icon = XUI::Pt(14.f);
			XUI::DrawIcon(ImGui::GetWindowDrawList(), XUI::Icon::Level, ImVec2(ImGui::GetCursorScreenPos().x, Snap((row_min.y + row_max.y - icon) * .5f)),
				14.f, XFinedTheme::U32(CT::TextSecondary));
			ImGui::Dummy(ImVec2(icon, ImGui::GetTextLineHeight()));
			ImGui::SameLine(0.f, XUI::Pt(6.f));
			const bool there = i < s_ImportInProject.size() && s_ImportInProject[i];
			XUI::TextEllipsis(name, _max(0.f, row_max.x - ImGui::GetCursorScreenPos().x - XUI::Pt(6.f) - (there ? badge + XUI::Pt(8.f) : 0.f)));
			if (there)
			{
				ImGui::SameLine(0.f, 0.f);
				ImGui::SetCursorScreenPos(ImVec2(row_max.x - XUI::Pt(6.f) - badge, ImGui::GetCursorScreenPos().y));
				XUI::Badge("In project");
			}
		}
		if (!shown)
			SecondaryText(s_ImportScenes.empty() ? "The editor library holds no scenes." : "No scene matches the search.");
	}
	XUI::EndList();
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
	char library[MAX_PATH * 3];
	AcpToUtf8(s_BaseMaps, library, sizeof(library));
	ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::U32(CT::TextMuted));
	XUI::TextEllipsis(library, -1.f, XUI::Clip::Start);
	ImGui::PopStyleColor();
	XUI::Tip("The editor's library of the game's source scenes. Importing copies one into the project; the library stays as it is.");

	char status[64];
	sprintf_s(status, "%u %s", u32(s_ImportScenes.size()), 1 == s_ImportScenes.size() ? "scene" : "scenes");
	if (XUI::BeginDialogFooter(status))
	{
		if (XUI::DialogButton("Import", XUI::Kind::Primary, true, !s_ImportPick.empty(), "Pick a scene first."))
			import = true;
		if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
		{
			s_WantImport = false;
			ImGui::CloseCurrentPopup();
		}
	}
	if (import && !s_ImportPick.empty())
	{
		bool there = false;
		for (u32 i = 0; i < s_ImportScenes.size() && i < s_ImportInProject.size(); ++i)
			if (s_ImportScenes[i] == s_ImportPick) there = !!s_ImportInProject[i];
		string_path dst;
		xr_string err;
		char name[MAX_PATH * 3], text[MAX_PATH * 4];
		AcpToUtf8(s_ImportPick.c_str(), name, sizeof(name));
		if (ImportBaseScene(s_ImportPick.c_str(), dst, err))
		{
			sprintf_s(text, there ? "Scene '%s' is in the project already." : "Scene '%s' imported into the project.", name);
			XUI::Toast(XUI::Tone::Success, text);
			s_WantImport = false;
			ImGui::CloseCurrentPopup();
		}
		else
		{
			AcpToUtf8(err.c_str(), text, sizeof(text));
			XUI::Notice(XUI::Tone::Error, "Import Base Scene", text);
		}
	}
	XUI::EndDialog();
}
