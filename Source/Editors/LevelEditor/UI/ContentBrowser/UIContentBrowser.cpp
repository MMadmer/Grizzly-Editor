#include "stdafx.h"
#include "..\..\..\XrECore\Editor\EditorGameContent.h"
#include "..\..\..\XrECore\Editor\EThumbnailVisual.h"
#include "..\..\..\XrECore\Editor\EDX11Utils.h"
#include "..\..\..\XrECore\Editor\EditorFileOps.h"
#include "..\..\..\XrECore\Editor\EditorModManifest.h"
#include "..\..\..\XrECore\Editor\Nq\NqExport.h"
#include "..\..\..\XrECore\Editor\Nq\NqLua.h"
#include "..\..\..\XrECore\Editor\Nq\NqUtil.h"
#include "..\..\..\XrECore\Editor\EditorCharacters.h"
#include "..\..\..\XrECore\Editor\EditorPda.h"
#include "..\..\..\XrECore\Editor\EditorAssetIcons.h"
#include "..\..\..\XrECore\Editor\EditorUiContract.h"
#include "..\QuestGraph\UIQuestGraph.h"
#include <shobjidl.h>
#include <shellapi.h>

using Token = XFinedTheme::ColorToken;

// image formats the thumbnail path can decode directly
static bool IsImageExt(LPCSTR name)
{
	LPCSTR ext = name ? strrchr(name, '.') : 0;
	return ext && (!_stricmp(ext, ".dds") || !_stricmp(ext, ".tga") || !_stricmp(ext, ".png") ||
				   !_stricmp(ext, ".jpg") || !_stricmp(ext, ".bmp"));
}

// a scene file, wherever it is listed from
static bool IsLevelExt(LPCSTR name)
{
	LPCSTR ext = name ? strrchr(name, '.') : 0;
	return ext && !_stricmp(ext, ".level");
}

// models carry no baked thumbnail, so their preview has to be rendered
static bool IsVisualExt(LPCSTR name)
{
	LPCSTR ext = name ? strrchr(name, '.') : 0;
	return ext && !_stricmp(ext, ".ogf");
}

// the editor's own model format - engine visuals are .ogf, sources are .object
static bool IsObjectExt(LPCSTR name)
{
	LPCSTR ext = name ? strrchr(name, '.') : 0;
	return ext && !_stricmp(ext, ".object");
}

// A graph the node editor opens: a quest (.nqasset) or a behaviour graph
// (.behasset) - docs/NQ_ARCHITECTURE.md par. 19. The extension is the kind.
static bool IsQuestExt(LPCSTR name)
{
	LPCSTR ext = name ? strrchr(name, '.') : 0;
	return ext && (!_stricmp(ext, ".nqasset") || !_stricmp(ext, ".behasset"));
}
// The module's own NPC (EditorCharacters.h)
static bool IsCharacterExt(LPCSTR name)
{
	LPCSTR ext = name ? strrchr(name, '.') : 0;
	return ext && !_stricmp(ext, ".npcasset");
}
// The module's own PDA interface (EditorPda.h)
static bool IsPdaExt(LPCSTR name)
{
	LPCSTR ext = name ? strrchr(name, '.') : 0;
	return ext && !_stricmp(ext, ".pdaasset");
}

// Which drawn icon a listing entry gets: every kind of the editor's own assets
// has one, and OwnExtension is the one list of them. The extension decides,
// except that a character template is told from a character by looking INSIDE
// the file, so the icons need a path that opens: the project listing keeps
// entries relative to the project root. Anywhere else the extension alone
// decides, which still picks the right family.
static EditorAssetIcons::EKind IconOf(LPCSTR entry, int source)
{
	if (!EditorAssetIcons::OwnExtension(entry))	return EditorAssetIcons::kNone;
	string_path disk;
	if (0 == source && EditorProject::Active() &&
		xr_strlen(EditorProject::Root()) + xr_strlen(entry) + 2 < sizeof(disk))
	{
		sprintf_s(disk, "%s\\%s", EditorProject::Root(), entry);
		return EditorAssetIcons::KindOf(disk);
	}
	return EditorAssetIcons::KindOf(entry);
}

// Nothing under the project root is registered in the editor FS: a project may
// live anywhere (My Documents by default) and only $maps$ is remounted into it,
// so FS.r_open on a project file always fails. Project bytes come off the disk
// directly - the same reason images already load through TextureTileFromFile.
static bool ReadProjectFile(LPCSTR rel, xr_vector<u8>& out)
{
	out.clear();
	if (!rel || !rel[0] || !EditorProject::Active()) return false;

	string_path abs;
	sprintf_s(abs, "%s\\%s", EditorProject::Root(), rel);
	HANDLE h = ::CreateFileA(abs, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING,
							 FILE_ATTRIBUTE_NORMAL, NULL);
	if (INVALID_HANDLE_VALUE == h) return false;

	LARGE_INTEGER size;
	if (!::GetFileSizeEx(h, &size) || size.QuadPart <= 0 || size.QuadPart > (64 << 20))
	{
		::CloseHandle(h);
		return false;
	}
	out.resize(size_t(size.QuadPart));
	DWORD read = 0;
	const bool ok = !!::ReadFile(h, out.data(), DWORD(out.size()), &read, NULL) && read == out.size();
	::CloseHandle(h);
	if (!ok) out.clear();
	return ok;
}

// "meshes\dynamics\box\box_1a.ogf" -> "dynamics\box\box_1a", the form
// ::Render->model_Create expects (relative to $game_meshes$, no extension)
static void ToVisualName(LPCSTR path, char* dst, u32 dst_size)
{
	LPCSTR src = path ? path : "";
	if (0 == _strnicmp(src, "meshes\\", 7))	src += 7;
	else if (0 == _strnicmp(src, "meshes/", 7)) src += 7;
	strncpy_s(dst, dst_size, src, _TRUNCATE);
	if (char* dot = strrchr(dst, '.')) *dot = 0;
	for (char* p = dst; *p; ++p) if (*p == '/') *p = '\\';
}

UIContentBrowser* UIContentBrowser::Form = nullptr;
xr_vector<UIContentBrowser::SLocation> UIContentBrowser::s_Favorites;
xr_string UIContentBrowser::s_FavoriteProject;
xr_string UIContentBrowser::s_FavoriteGameRoot;

struct UIContentBrowser::SDirectoryWatcher
{
	HANDLE			directory;
	HANDLE			event;
	OVERLAPPED		io;
	xr_vector<u8>		buffer;
	xr_string			root;
	int				source;
	ULONGLONG			retry_after;
	ULONGLONG			last_change;
	bool				armed;
	bool				refresh_pending;

	SDirectoryWatcher()
		: directory(INVALID_HANDLE_VALUE), event(0), source(-1), retry_after(0),
		  last_change(0), armed(false), refresh_pending(false)
	{
		ZeroMemory(&io, sizeof(io));
		buffer.resize(64 * 1024);
	}
};

// filled in by the preview window when it is compiled into the build
static UIContentBrowser::TShowVisual	s_ShowVisual	= 0;
static UIContentBrowser::TShowVisualMem	s_ShowVisualMem	= 0;
static UIContentBrowser::TShowVisualMem	s_ShowObjectMem	= 0;

void UIContentBrowser::SetVisualPreview(TShowVisual by_name, TShowVisualMem from_memory,
										TShowVisualMem object_from_memory)
{
	s_ShowVisual	= by_name;
	s_ShowVisualMem	= from_memory;
	s_ShowObjectMem	= object_from_memory;
}

// Categories exposed by the browser. Ids come straight from EChooseMode, so the
// fill/thumbnail delegates registered in FillChooseEvents are reused verbatim.
struct SCategory { u32 id; LPCSTR caption; bool placeable; };
// Levels have no EChooseMode - they are scenes, not library assets - so they get
// a sentinel id the browser special-cases when filling and when opening.
static const u32 kLevelsCategoryId = u32(-2);
static const SCategory kCategories[] =
{
	{ smObject,		"Objects",		true  },
	{ kLevelsCategoryId, "Levels",	false },
	{ smGroup,		"Groups",		false },
	{ smVisual,		"Visuals",		false },
	{ smTexture,	"Textures",		false },
	{ smTextureRaw,	"Textures (raw)",false },
	{ smPE,			"Particles",	false },
	{ smSoundSource,"Sounds",		false },
	{ smEntityType,	"Entities",		false },
	{ smLAnim,		"Light Anims",	false },
};
static const int kCategoryCount = sizeof(kCategories)/sizeof(kCategories[0]);

// Which files back an "Editor Content" item, per category. An item there is a
// library REFERENCE without an extension, so copying one into the project means
// resolving it against the fs.ltx aliases first. More than one entry per row is
// normal and wanted: a texture is only useful in a mod with its .thm beside it,
// and the uncompressed source is worth taking when it is there.
struct SLibFile { LPCSTR alias; LPCSTR ext; };
static const SLibFile kLibObject[]	= { { "$objects$",		".object"	} };
static const SLibFile kLibGroup[]	= { { "$groups$",		".group"	} };
static const SLibFile kLibVisual[]	= { { "$game_meshes$",	".ogf"		},
										{ "$game_meshes$",	".omf"		} };
static const SLibFile kLibTexture[]	= { { "$game_textures$",".dds"		},
										{ "$textures$",		".thm"		},
										{ "$textures$",		".tga"		} };
static const SLibFile kLibSound[]	= { { "$game_sounds$",	".ogg"		},
										{ "$sounds$",		".thm"		},
										{ "$sounds$",		".wav"		} };
static const SLibFile kLibLevel[]	= { { "$maps$",			".level"	} };

static const SLibFile* LibFilesFor(u32 category, int& count)
{
#define LIB_ROW(v)	do { count = sizeof(v)/sizeof(v[0]); return v; } while(0)
	if (category == kLevelsCategoryId)	LIB_ROW(kLibLevel);
	switch (category)
	{
	case smObject:		LIB_ROW(kLibObject);
	case smGroup:		LIB_ROW(kLibGroup);
	case smVisual:		LIB_ROW(kLibVisual);
	case smTexture:
	case smTextureRaw:	LIB_ROW(kLibTexture);
	case smSoundSource:	LIB_ROW(kLibSound);
	}
#undef LIB_ROW
	// particles, light animations and entity types live inside one shared file
	// each - there is no per-item file to hand over
	count = 0;
	return 0;
}

// keep at most this many live thumbnail textures
static const u32 kThumbBudget = 512;
// The milliseconds one frame may spend decoding pictures, one always goes: a game folder of 2048 px UI atlases fills
// over a few frames instead of stopping one. Tile submission is virtualized below.
static const u32 kThumbDecodeMs = 12;
// a list row (pt), the picture at its start (pt) and the narrowest column the rows are laid out in (em)
static const float kListRow			= 22.f;
static const float kListPicture		= 18.f;
static const float kListColumnEm	= 12.f;
// between two tiles, and around a tile's square inside it (pt)
static const float kTileGap			= 8.f;
static const float kTilePad			= 4.f;
// the grid's own padding above and below the tiles (pt)
static const float kGridPadY		= 4.f;
static const u32 kNavigationHistoryLimit = 64;

static bool SameLocation(const UIContentBrowser::SLocation& lhs,
	const UIContentBrowser::SLocation& rhs)
{
	return lhs.source == rhs.source && (lhs.source != 1 || lhs.category == rhs.category) &&
		0 == _stricmp(lhs.folder.c_str(), rhs.folder.c_str());
}

static void RemoveScopedLocations(xr_vector<UIContentBrowser::SLocation>& locations,
	bool remove_project, bool remove_darf)
{
	for (u32 i = 0; i < locations.size();)
	{
		const int source = locations[i].source;
		if ((remove_project && source == 0) || (remove_darf && source == 2))
			locations.erase(locations.begin() + i);
		else
			++i;
	}
}

static bool NormalizeFolderPath(LPCSTR input, xr_string& out, xr_string& err)
{
	const xr_string raw = input ? input : "";
	out.clear();
	err.clear();
	if (raw.empty()) return true;

	xr_string part;
	for (LPCSTR p = raw.c_str();; ++p)
	{
		const char ch = *p;
		if (ch && ch != '\\' && ch != '/')
		{
			if (strchr(":*?\"<>|", ch))
			{
				err = "folder contains an invalid character";
				return false;
			}
			part += ch;
			continue;
		}

		if (!part.empty())
		{
			if (part == "." || part == "..")
			{
				err = "folder traversal is not allowed";
				return false;
			}
			if (!out.empty()) out += "\\";
			out += part;
			part.clear();
		}
		if (!ch) break;
	}

	if (out.size() >= sizeof(string_path))
	{
		err = "folder path is too long";
		return false;
	}
	return true;
}

static ImU32 ThemeCol(Token token, float alpha = 1.f)
{
	return XFinedTheme::U32(token, alpha);
}

// a coordinate on the pixel grid (imgui_internal's IM_ROUND stays inside XrEUI)
static float PxRound(float v)
{
	return floorf(v + .5f);
}

static xr_string Span(LPCSTR begin, LPCSTR end)
{
	xr_string out;
	out.assign(begin, end);
	return out;
}

// The glyph a tile draws while it has no picture: the editor's own kinds first, then the file type. An
// Editor Content entry is a library reference without an extension, so its category says what it is.
static XUI::Icon TileIcon(LPCSTR entry, bool folder, int source, u32 category)
{
	if (folder)
		return XUI::Icon::Folder;
	if (const EditorAssetIcons::EKind kind = IconOf(entry, source))
		return EditorAssetIcons::IconFor(kind);
	LPCSTR leaf = strrchr(entry, '\\');
	LPCSTR ext = strrchr(leaf ? leaf : entry, '.');
	if (!ext && 1 == source)
	{
		if (kLevelsCategoryId == category)
			return XUI::Icon::Level;
		switch (category)
		{
		case smObject:		return XUI::Icon::SceneObject;
		case smGroup:		return XUI::Icon::Group;
		case smVisual:		return XUI::Icon::FileModel;
		case smTexture:
		case smTextureRaw:	return XUI::Icon::FileTexture;
		case smPE:			return XUI::Icon::Particles;
		case smSoundSource:	return XUI::Icon::FileSound;
		case smEntityType:	return XUI::Icon::Spawn;
		case smLAnim:		return XUI::Icon::Light;
		}
		return XUI::Icon::File;
	}
	if (!ext)
		return XUI::Icon::File;
	struct SType { LPCSTR ext; XUI::Icon icon; };
	static const SType kTypes[] =
	{
		{ ".dds", XUI::Icon::FileTexture }, { ".tga", XUI::Icon::FileTexture }, { ".png", XUI::Icon::FileTexture },
		{ ".jpg", XUI::Icon::FileTexture }, { ".jpeg", XUI::Icon::FileTexture }, { ".bmp", XUI::Icon::FileTexture },
		{ ".ogf", XUI::Icon::FileModel }, { ".object", XUI::Icon::FileModel }, { ".omf", XUI::Icon::FileModel },
		{ ".ogg", XUI::Icon::FileSound }, { ".wav", XUI::Icon::FileSound },
		{ ".script", XUI::Icon::FileScript }, { ".lua", XUI::Icon::FileScript },
		{ ".ltx", XUI::Icon::FileConfig }, { ".ltxp", XUI::Icon::FileConfig }, { ".ini", XUI::Icon::FileConfig },
		{ ".xml", XUI::Icon::FileXml }, { ".xmlp", XUI::Icon::FileXml },
		{ ".txt", XUI::Icon::FileText }, { ".md", XUI::Icon::FileText }, { ".log", XUI::Icon::FileText },
		{ ".seq", XUI::Icon::FileText },
		{ ".zip", XUI::Icon::FileArchive }, { ".7z", XUI::Icon::FileArchive }, { ".rar", XUI::Icon::FileArchive },
		{ ".level", XUI::Icon::Level }, { ".group", XUI::Icon::Group },
	};
	for (const SType& type : kTypes)
		if (!_stricmp(ext, type.ext))
			return type.icon;
	// the game's archives: .db0 ... .db9, .xdb0 ..., gamedata.dbN
	if (!_strnicmp(ext, ".db", 3) || !_strnicmp(ext, ".xdb", 4))
		return XUI::Icon::FileArchive;
	return XUI::Icon::File;
}

// A glyph the way the editor's own asset icons sit in a tile (EditorAssetIcons::Draw): 58% of the square,
// stopping at 48 pt, where the stroke only gets heavier.
static void DrawTileGlyph(ImDrawList* dl, XUI::Icon icon, const ImVec2& a, const ImVec2& b, ImU32 col)
{
	const float side = _min(_min(b.x - a.x, b.y - a.y) * .58f * 16.f / 14.f, XUI::Pt(48.f));
	if (side < 4.f)
		return;
	const ImVec2 at(PxRound((a.x + b.x - side) * .5f), PxRound((a.y + b.y - side) * .5f));
	XUI::DrawIconRect(dl, icon, at, ImVec2(at.x + side, at.y + side), col);
}

// The checkerboard every image editor puts under transparency. Without it a texture that is mostly clear - a UI atlas,
// a cursor - is a dark square nobody tells from a tile with no picture. Cells of 8 pt, down to 4 in a small picture.
static void DrawChecker(ImDrawList* dl, const ImVec2& a, const ImVec2& b)
{
	const float w = b.x - a.x, h = b.y - a.y;
	if (w < 1.f || h < 1.f)
		return;
	const float cell = _max(2.f, PxRound(clampr(_max(w, h) / 8.f, XUI::Pt(4.f), XUI::Pt(8.f))));
	dl->AddRectFilled(a, b, ThemeCol(Token::Border));
	const ImU32 light = ThemeCol(Token::BorderStrong);
	int row = 0;
	for (float y = a.y; y < b.y; y += cell, ++row)
		for (float x = a.x + ((row & 1) ? cell : 0.f); x < b.x; x += cell * 2.f)
			dl->AddRectFilled(ImVec2(x, y), ImVec2(_min(x + cell, b.x), _min(y + cell, b.y)), light);
}

// A picture fitted into a..b, never stretched, transparency on the checkerboard
static void DrawPicture(ImDrawList* dl, ImTextureID tex, float aspect, bool alpha, const ImVec2& a, const ImVec2& b)
{
	const float w = b.x - a.x, h = b.y - a.y;
	if (w < 1.f || h < 1.f || aspect <= 0.f)
		return;
	// the box is wider than the picture: its height sets the size
	const bool by_height = w / h > aspect;
	const float fw = by_height ? h * aspect : w;
	const float fh = by_height ? h : w / aspect;
	const ImVec2 fa(PxRound(a.x + (w - fw) * .5f), PxRound(a.y + (h - fh) * .5f));
	const ImVec2 fb(fa.x + _max(1.f, PxRound(fw)), fa.y + _max(1.f, PxRound(fh)));
	if (alpha)
		DrawChecker(dl, fa, fb);
	dl->AddImage(tex, fa, fb);
}

static void ReleaseThumbTexture(ImTextureID tex)
{
	if (!tex)
		return;
#if defined(USE_DX11)
	// ImTextureID is already the view type here - no cast needed
	tex->Release();
#else
	((IDirect3DBaseTexture9*)tex)->Release();
#endif
}

// One UTF-8 character's length from its lead byte
static int Utf8Len(char lead)
{
	const u8 c = u8(lead);
	return c < 0xC0 ? 1 : c < 0xE0 ? 2 : c < 0xF0 ? 3 : 4;
}

static const char* const kEllipsis = "\xE2\x80\xA6";

UIContentBrowser::UIContentBrowser()
{
	m_Watcher		= xr_new<SDirectoryWatcher>();
	m_Category		= 0;
	// the project's own content is what someone opening the editor is working
	// on; the shared library is a place you go to fetch from
	m_Source		= 0;
	m_NeedRefresh	= true;
	m_TileSize[kHostDrawer]		= 0.f;
	m_TileSize[kHostPanel]		= 0.f;
	m_GridHeight[kHostDrawer]	= 0.f;
	m_GridHeight[kHostPanel]	= 0.f;
	m_ListView[kHostDrawer]	= false;
	m_ListView[kHostPanel]	= true;
	m_Host			= kHostDrawer;
	m_PanelFrame	= -1;
	m_PanelLive		= false;
	m_RaiseFrames	= 0;
	m_Tick			= 0;
	m_ClipSource		= -1;
	m_ClipCategory		= u32(-1);
	m_ClipCut			= false;
	m_HasAnchor			= false;
	m_HasPendingRange	= false;
	m_ViewDirty			= true;
	m_ScrollToSelection	= false;
	m_RenameBuf[0]		= 0;
	m_RenameFocus		= false;
	m_RenameSelect		= -1;
	m_RenameStem		= 0;
	m_Marquee		= false;
	m_MarqueeStart	= ImVec2(0, 0);
	m_ViewLoaded	= false;
	m_SourcesPane	= true;
	m_SourcesFit	= true;
	m_SourcesShown	= false;
	m_Search[0]		= 0;
	m_ViewFolders	= 0;
	m_ViewFiles		= 0;
	m_Root.name		= "Content";
	m_Root.path		= "";
	m_DarfReady		= false;
	m_ThumbsThisFrame = 0;
}

UIContentBrowser::~UIContentBrowser()
{
	StopWatcher();
	xr_delete(m_Watcher);
	DropCache();
}

void UIContentBrowser::Show()
{
	if (!Form) Form = xr_new<UIContentBrowser>();
	Form->bOpen = true;
}

void UIContentBrowser::Close()
{
	xr_delete(Form);
}

void UIContentBrowser::Tick()
{
	if (!Form)
		return;
	Form->UpdateWatcher();
	// Runs after the frame's panels. Taken while Level is up and kept while a document is: activating Level and
	// asking for the browser before a frame has passed still finds the panel it will draw. A frame of slack
	// covers a panel drawn after this.
	if (XUI::Shell::IsActive("level"))
		Form->m_PanelLive = Form->bOpen && Form->m_PanelFrame + 1 >= ImGui::GetFrameCount();
}

void UIContentBrowser::DrawPanel()
{
	// a closed panel keeps the browser: the drawer shows the same one
	if (Form && Form->bOpen)
		Form->Draw();
}

void UIContentBrowser::DrawDrawer()
{
	if (!Form)
	{
		Form = xr_new<UIContentBrowser>();
		Form->bOpen = false;
	}
	Form->m_Host = kHostDrawer;
	Form->DrawBody();
}

bool UIContentBrowser::PanelLive()
{
	return Form && Form->bOpen && Form->m_PanelLive && XUI::Shell::IsActive("level");
}

int UIContentBrowser::TargetHost()
{
	return PanelLive() ? kHostPanel : kHostDrawer;
}

void UIContentBrowser::MakeVisible()
{
	if (!Form)
	{
		Form = xr_new<UIContentBrowser>();
		Form->bOpen = false;
	}
	// Level draws the panel - docked in the strip, possibly behind the Output Log's tab, or floating: that
	// is where it comes up, never a drawer over it
	if (PanelLive())
	{
		XUI::Shell::CloseDrawer();
		XUI::Shell::FocusWindow(PanelName());
		Form->m_RaiseFrames = 2;
		return;
	}
	XUI::Shell::OpenDrawer(XUI::Shell::DrawerContent);
}

LPCSTR UIContentBrowser::VisibleHost()
{
	if (XUI::Shell::OpenedDrawer() == XUI::Shell::DrawerContent)
		return "drawer";
	if (PanelLive() && (Form->m_RaiseFrames > 0 || XUI::Shell::WindowShown(PanelName())))
		return "panel";
	return "none";
}

LPCSTR UIContentBrowser::DraggedAsset()
{
	return (Form && !Form->m_Dragged.empty()) ? Form->m_Dragged.c_str() : 0;
}

u32 UIContentBrowser::DraggedCategory()
{
	return Form ? kCategories[Form->m_Category].id : 0;
}

int UIContentBrowser::CategoryCount() { return kCategoryCount; }

LPCSTR UIContentBrowser::CategoryName(int index)
{
	return (index >= 0 && index < kCategoryCount) ? kCategories[index].caption : "";
}

u32 UIContentBrowser::CategoryId(int index)
{
	return (index >= 0 && index < kCategoryCount) ? kCategories[index].id : 0;
}

bool UIContentBrowser::IsLevelsCategory(int index)
{
	return (index >= 0 && index < kCategoryCount) && (kLevelsCategoryId == kCategories[index].id);
}

int UIContentBrowser::FindCategory(LPCSTR name)
{
	if (!name || !name[0]) return -1;
	for (int i = 0; i < kCategoryCount; ++i)
		if (0 == _stricmp(kCategories[i].caption, name))
			return i;
	return -1;
}

UIContentBrowser::SLocation UIContentBrowser::CurrentLocation() const
{
	return SLocation(m_Source, int(m_Category), m_CurFolder.c_str());
}

void UIContentBrowser::PushHistory(xr_vector<SLocation>& history, const SLocation& location)
{
	if (!history.empty() && SameLocation(history.back(), location)) return;
	if (history.size() >= kNavigationHistoryLimit) history.erase(history.begin());
	history.push_back(location);
}

void UIContentBrowser::RememberNavigation(const SLocation& origin)
{
	if (SameLocation(origin, CurrentLocation())) return;
	PushHistory(m_BackHistory, origin);
	m_ForwardHistory.clear();
}

bool UIContentBrowser::ApplyLocation(SLocation location, bool record, xr_string& err)
{
	err.clear();
	if (location.source < 0) location.source = m_Source;
	if (location.category < 0) location.category = int(m_Category);
	if (location.source < 0 || location.source > 2)
	{
		err = "source must be 0 (project), 1 (editor), or 2 (DARF)";
		return false;
	}
	if (location.category < 0 || location.category >= kCategoryCount)
	{
		err = "asset category is out of range";
		return false;
	}
	if (!NormalizeFolderPath(location.folder.c_str(), location.folder, err)) return false;
	if (location.source == 0 && !EditorProject::Active())
	{
		err = "no project is open";
		return false;
	}

	EnsureListing();
	const SLocation origin = CurrentLocation();
	if (SameLocation(origin, location)) return true;

	const bool rebuild = location.source != m_Source || u32(location.category) != m_Category;
	if (rebuild)
	{
		m_Source = location.source;
		m_Category = u32(location.category);
		m_NeedRefresh = true;
		if (m_Source == 2)
		{
			m_DarfReady = false;
			m_DarfStatus = "Reading the linked game.";
		}
		EnsureListing();
	}

	SFolder* folder = FindFolder(location.folder.c_str());
	// a folder something else made a moment ago - an MCP write, a checkout - is one more look
	// at the disk away; the watcher would get there too, only later
	if (!folder && 0 == m_Source)
	{
		Refresh();
		folder = FindFolder(location.folder.c_str());
	}
	if (!folder)
	{
		if (rebuild)
		{
			m_Source = origin.source;
			m_Category = u32(origin.category);
			m_NeedRefresh = true;
			Refresh();
			m_CurFolder = origin.folder;
		}
		err = "folder is not available in this source";
		return false;
	}

	location.folder = folder->path;
	if (record)
	{
		PushHistory(m_BackHistory, origin);
		m_ForwardHistory.clear();
	}
	m_CurFolder = location.folder;
	InvalidateView();
	ClearSelection();
	m_Rename.clear();
	m_RenameFocus = false;
	m_ScrollToSelection = false;
	return true;
}

bool UIContentBrowser::GoBack(xr_string& err)
{
	EnsureListing();
	const SLocation current = CurrentLocation();
	xr_string last_error;
	while (!m_BackHistory.empty())
	{
		const SLocation target = m_BackHistory.back();
		if (ApplyLocation(target, false, err))
		{
			if (!m_BackHistory.empty() && SameLocation(m_BackHistory.back(), target))
				m_BackHistory.pop_back();
			PushHistory(m_ForwardHistory, current);
			return true;
		}
		last_error = err;
		if (!m_BackHistory.empty() && SameLocation(m_BackHistory.back(), target))
			m_BackHistory.pop_back();
	}
	err = last_error.empty() ? "back history is empty" : last_error;
	return false;
}

bool UIContentBrowser::GoForward(xr_string& err)
{
	EnsureListing();
	const SLocation current = CurrentLocation();
	xr_string last_error;
	while (!m_ForwardHistory.empty())
	{
		const SLocation target = m_ForwardHistory.back();
		if (ApplyLocation(target, false, err))
		{
			if (!m_ForwardHistory.empty() && SameLocation(m_ForwardHistory.back(), target))
				m_ForwardHistory.pop_back();
			PushHistory(m_BackHistory, current);
			return true;
		}
		last_error = err;
		if (!m_ForwardHistory.empty() && SameLocation(m_ForwardHistory.back(), target))
			m_ForwardHistory.pop_back();
	}
	err = last_error.empty() ? "forward history is empty" : last_error;
	return false;
}

bool UIContentBrowser::GoUp(xr_string& err)
{
	// the root has no parent: already where Up leads, which is no failure
	err.clear();
	if (m_CurFolder.empty())
		return true;

	xr_string parent = m_CurFolder;
	if (LPCSTR cut = strrchr(parent.c_str(), '\\'))
		parent.erase(size_t(cut - parent.c_str()));
	else
		parent.clear();
	return ApplyLocation(SLocation(m_Source, int(m_Category), parent.c_str()), true, err);
}

bool UIContentBrowser::GoHome(xr_string& err)
{
	// at the root already this changes nothing and succeeds
	return ApplyLocation(SLocation(m_Source, int(m_Category), ""), true, err);
}

void UIContentBrowser::SyncFavoriteScopes()
{
	LPCSTR project_root = EditorProject::Active() ? EditorProject::Root() : "";
	LPCSTR game_root = EditorProject::GameRoot();
	const bool project_changed = 0 != _stricmp(s_FavoriteProject.c_str(), project_root);
	const bool game_changed = 0 != _stricmp(s_FavoriteGameRoot.c_str(), game_root);

	if (project_changed || game_changed)
		RemoveScopedLocations(s_Favorites, project_changed, project_changed || game_changed);

	s_FavoriteProject = project_root;
	s_FavoriteGameRoot = game_root;
}

void UIContentBrowser::GetHistory(SLocation& current, xr_vector<SLocation>& back,
	xr_vector<SLocation>& forward)
{
	SyncFavoriteScopes();
	if (Form)
	{
		Form->EnsureListing();
		current = Form->CurrentLocation();
		back = Form->m_BackHistory;
		forward = Form->m_ForwardHistory;
		return;
	}
	current = SLocation(-1, -1, "");
	back.clear();
	forward.clear();
}

bool UIContentBrowser::NavigateToFolder(int source, int category, LPCSTR folder, xr_string& err)
{
	Show();
	if (!Form)
	{
		err = "content browser unavailable";
		return false;
	}
	return Form->ApplyLocation(SLocation(source, category, folder), true, err);
}

bool UIContentBrowser::NavigateBack(xr_string& err)
{
	Show();
	if (!Form) { err = "content browser unavailable"; return false; }
	return Form->GoBack(err);
}

bool UIContentBrowser::NavigateForward(xr_string& err)
{
	Show();
	if (!Form) { err = "content browser unavailable"; return false; }
	return Form->GoForward(err);
}

bool UIContentBrowser::NavigateUp(xr_string& err)
{
	Show();
	if (!Form) { err = "content browser unavailable"; return false; }
	return Form->GoUp(err);
}

bool UIContentBrowser::NavigateHome(xr_string& err)
{
	Show();
	if (!Form) { err = "content browser unavailable"; return false; }
	return Form->GoHome(err);
}

void UIContentBrowser::RefreshNow()
{
	Show();
	if (!Form) return;
	Form->m_NeedRefresh = true;
	Form->EnsureListing();
}

int UIContentBrowser::FindFavorite(const SLocation& location) const
{
	for (u32 i = 0; i < s_Favorites.size(); ++i)
		if (SameLocation(s_Favorites[i], location)) return int(i);
	return -1;
}

bool UIContentBrowser::ValidateFavorite(SLocation& location, xr_string& err)
{
	if (location.source == m_Source &&
		(location.source != 1 || u32(location.category) == m_Category))
	{
		EnsureListing();
		SFolder* folder = FindFolder(location.folder.c_str());
		if (!folder)
		{
			err = "folder is not available in this source";
			return false;
		}
		location.folder = folder->path;
		return true;
	}

	if (location.source == 0)
	{
		if (!EditorProject::Active())
		{
			err = "no project is open";
			return false;
		}
		if (location.folder.empty()) return true;

		xr_string top = location.folder;
		if (const size_t cut = top.find('\\'); cut != xr_string::npos) top.erase(cut);
		if (EditorProject::IsEditorOnlyEntry(top.c_str()))
		{
			err = "folder is editor-owned, not project content";
			return false;
		}

		string_path absolute;
		const size_t absolute_length = xr_strlen(EditorProject::Root()) + 1 + location.folder.size();
		if (absolute_length >= sizeof(absolute))
		{
			err = "project favorite path is too long";
			return false;
		}
		xr_sprintf(absolute, sizeof(absolute), "%s\\%s", EditorProject::Root(), location.folder.c_str());
		const DWORD attributes = ::GetFileAttributesA(absolute);
		if (attributes != INVALID_FILE_ATTRIBUTES && (attributes & FILE_ATTRIBUTE_DIRECTORY)) return true;
		err = "folder is not available in this source";
		return false;
	}

	if (location.source == 2)
	{
		xr_string mount_error;
		if (!EditorGameContent::EnsureMounted(mount_error))
		{
			err = mount_error;
			return false;
		}
		if (location.folder.empty()) return true;

		const size_t length = location.folder.size();
		for (int i = 0; i < EditorGameContent::Count(); ++i)
		{
			LPCSTR path = EditorGameContent::Get(i)->path.c_str();
			if (xr_strlen(path) > length && 0 == _strnicmp(path, location.folder.c_str(), length) &&
				path[length] == '\\')
				return true;
		}
		err = "folder is not available in this source";
		return false;
	}

	if (location.folder.empty()) return true;
	ChooseItemVec items;
	if (kCategories[location.category].id == kLevelsCategoryId)
	{
		FS_FileSet levels;
		if (FS.file_list(levels, _maps_, FS_ListFiles | FS_ClampExt, "*.level"))
			for (FS_FileSetIt it = levels.begin(); it != levels.end(); ++it)
				items.push_back(SChooseItem(it->name.c_str(), ""));
		xr_vector<xr_string> base;
		EditorProject::ListBaseScenes(base);
		for (u32 i = 0; i < base.size(); ++i) items.push_back(SChooseItem(base[i].c_str(), ""));
	}
	else if (SChooseEvents* events = UIChooseForm::GetEvents(kCategories[location.category].id);
		events && events->on_fill)
	{
		events->on_fill(items, 0);
	}

	const size_t length = location.folder.size();
	for (u32 i = 0; i < items.size(); ++i)
	{
		LPCSTR path = items[i].name.c_str();
		if (xr_strlen(path) > length && 0 == _strnicmp(path, location.folder.c_str(), length) &&
			path[length] == '\\')
			return true;
	}
	err = "folder is not available in this source";
	return false;
}

void UIContentBrowser::GetFavorites(xr_vector<SLocation>& out)
{
	SyncFavoriteScopes();
	out = s_Favorites;
}

bool UIContentBrowser::AddFavorite(SLocation location, xr_string& err)
{
	Show();
	if (!Form) { err = "content browser unavailable"; return false; }
	Form->EnsureListing();
	if (location.source < 0) location.source = Form->m_Source;
	if (location.category < 0) location.category = int(Form->m_Category);
	if (location.source < 0 || location.source > 2)
	{
		err = "source must be 0 (project), 1 (editor), or 2 (DARF)";
		return false;
	}
	if (location.category < 0 || location.category >= kCategoryCount)
	{
		err = "asset category is out of range";
		return false;
	}
	if (!NormalizeFolderPath(location.folder.c_str(), location.folder, err)) return false;
	if (location.source == 0 && !EditorProject::Active())
	{
		err = "no project is open";
		return false;
	}
	if (Form->FindFavorite(location) >= 0)
	{
		err = "folder is already a favorite";
		return false;
	}

	if (!Form->ValidateFavorite(location, err)) return false;

	s_Favorites.push_back(location);
	err.clear();
	return true;
}

bool UIContentBrowser::RemoveFavorite(int index, xr_string& err)
{
	SyncFavoriteScopes();
	if (index < 0 || index >= int(s_Favorites.size()))
	{
		err = "favorite index is out of range";
		return false;
	}
	s_Favorites.erase(s_Favorites.begin() + index);
	err.clear();
	return true;
}

bool UIContentBrowser::NavigateFavorite(int index, xr_string& err)
{
	Show();
	SyncFavoriteScopes();
	if (Form) Form->EnsureListing();
	if (!Form || index < 0 || index >= int(s_Favorites.size()))
	{
		err = "favorite index is out of range";
		return false;
	}
	return Form->ApplyLocation(s_Favorites[index], true, err);
}

void UIContentBrowser::DropCache()
{
	for (ThumbMapIt it = m_Thumbs.begin(); it != m_Thumbs.end(); ++it)
		ReleaseThumbTexture(it->second.tex);
	m_Thumbs.clear();
	ClearVisualThumbnailQueue();
}

//------------------------------------------------------------------------------
// data
//------------------------------------------------------------------------------
// `dirs` collects the folders themselves: an empty one has no items to derive it
// from, and a folder the user just created is empty by definition.
static bool ContentAbsolutePathFits(LPCSTR base, size_t relative_length)
{
	return xr_strlen(base) + 1 + relative_length < sizeof(string_path);
}

// A tile caption over up to `max_lines` lines, each centred on the tile (`centre`; a list row's name starts at the
// left). It breaks where a name breaks for a reader: after a space, '_', '-', '.' or ',', and between the words of a
// CamelCase name (NewBehaviour -> New / Behaviour); mid-word only when one word is wider than the tile. Never right
// before the extension: "bytil_wnd.dds" goes as "bytil_" / "wnd.dds". A last line that does not fit
// keeps the extension and loses the middle of the name - the tooltip has all of it.
static void DrawTileCaption(ImDrawList* dl, ImFont* font, float size, const ImVec2& p, float width, int max_lines,
							ImU32 col, LPCSTR text, LPCSTR end, bool has_ext, bool centre)
{
	if (!end) end = text + xr_strlen(text);
	auto width_of = [&](LPCSTR a, LPCSTR b) { return font->CalcTextSizeA(size, FLT_MAX, 0.f, a, b).x; };
	auto put = [&](int line, LPCSTR a, LPCSTR b)
	{
		const float x = centre ? p.x + _max(0.f, (width - width_of(a, b)) * .5f) : p.x;
		dl->AddText(font, size, ImVec2(PxRound(x), PxRound(p.y + line * size)), col, a, b);
	};
	// where the extension starts: no line may begin at it or inside it
	LPCSTR ext = nullptr;
	if (has_ext)
		for (LPCSTR q = end - 1; q > text; --q)
			if ('.' == *q)
			{
				ext = q;
				break;
			}
	auto lower = [](char c) { return (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9'); };
	auto upper = [](char c) { return c >= 'A' && c <= 'Z'; };

	LPCSTR start = text;
	for (int line = 0; line < max_lines && start < end; ++line)
	{
		while (start < end && ' ' == *start) ++start;
		if (start >= end) break;
		if (width_of(start, end) <= width)
		{
			put(line, start, end);
			break;
		}
		if (line == max_lines - 1)
		{
			// the rest does not fit: the extension stays whole, the name gives up its middle
			const bool keep_ext = ext && ext > start;
			const float tail_w = keep_ext ? width_of(ext, end) : 0.f;
			const float room = width - width_of(kEllipsis, nullptr) - tail_w;
			LPCSTR head_end = keep_ext ? ext : end;
			LPCSTR fit = start;
			if (room > 0.f)
				font->CalcTextSizeA(size, room, 0.f, start, head_end, &fit);
			xr_string shown = Span(start, fit);
			shown += kEllipsis;
			if (keep_ext && room > 0.f)
				shown.append(ext, end);
			else if (keep_ext)
			{
				// not even the extension fits: the end goes instead
				shown.clear();
				font->CalcTextSizeA(size, _max(0.f, width - width_of(kEllipsis, nullptr)), 0.f, start, end, &fit);
				shown.assign(start, fit);
				shown += kEllipsis;
			}
			put(line, shown.c_str(), shown.c_str() + shown.size());
			XUI::AuditTruncated(Span(text, end).c_str(), shown.c_str(), ImVec2(p.x, p.y + line * size),
				ImVec2(p.x + width, p.y + (line + 1) * size));
			break;
		}
		// what fits, then the last place a reader would break it
		LPCSTR fit = start;
		font->CalcTextSizeA(size, width, 0.f, start, end, &fit);
		if (fit <= start)
			fit = _min(end, start + Utf8Len(*start));
		LPCSTR brk = nullptr;
		for (LPCSTR at = start; at < fit;)
		{
			LPCSTR next = _min(end, at + Utf8Len(*at));
			if (next <= fit && next < end && (!ext || next < ext) &&
				(strchr(" _-.,", *at) || (lower(*at) && upper(*next))))
				brk = next;
			at = next;
		}
		LPCSTR cut = brk ? brk : fit;
		// a word wider than the tile is cut where it stops fitting - still short of its extension
		if (ext && cut >= ext && !brk)
		{
			LPCSTR back = ext - 1;
			while (back > start && (u8(*back) & 0xC0) == 0x80) --back;
			if (back > start)
				cut = back;
		}
		put(line, start, cut);
		start = cut;
	}
}

static void ScanContentDir(const char* base, const char* rel, ChooseItemVec& out,
						   xr_vector<xr_string>& dirs)
{
	char mask[MAX_PATH];
	const size_t base_length = xr_strlen(base);
	const size_t rel_length = xr_strlen(rel);
	if (base_length + 1 + rel_length + (rel_length ? 1 : 0) + 1 >= sizeof(mask)) return;
	sprintf_s(mask, "%s\\%s%s*", base, rel, rel[0] ? "\\" : "");
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (h == INVALID_HANDLE_VALUE) return;
	do
	{
		if (!strcmp(fd.cFileName, ".") || !strcmp(fd.cFileName, "..")) continue;
		// version control and tool litter: nobody's content, and a .git alone is
		// thousands of entries walked on every refresh
		if (EditorProject::IsToolingEntry(fd.cFileName)) continue;
		const size_t sub_length = rel_length + (rel_length ? 1 : 0) + xr_strlen(fd.cFileName);
		if (sub_length >= sizeof(string_path) || !ContentAbsolutePathFits(base, sub_length)) continue;
		if (fd.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) continue;
		char sub[MAX_PATH];
		sprintf_s(sub, "%s%s%s", rel, rel[0] ? "\\" : "", fd.cFileName);
		if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)
		{
			dirs.push_back(xr_string(sub));
			ScanContentDir(base, sub, out, dirs);
		}
		else
			out.push_back(SChooseItem(sub, ""));
	} while (::FindNextFileA(h, &fd));
	::FindClose(h);
}

void UIContentBrowser::CloseWatcherHandles()
{
	if (!m_Watcher) return;
	SDirectoryWatcher& watcher = *m_Watcher;
	if (watcher.directory != INVALID_HANDLE_VALUE)
	{
		if (watcher.armed)
		{
			::CancelIoEx(watcher.directory, &watcher.io);
			DWORD ignored = 0;
			::GetOverlappedResult(watcher.directory, &watcher.io, &ignored, TRUE);
		}
		::CloseHandle(watcher.directory);
	}
	if (watcher.event) ::CloseHandle(watcher.event);
	watcher.directory = INVALID_HANDLE_VALUE;
	watcher.event = 0;
	watcher.armed = false;
	ZeroMemory(&watcher.io, sizeof(watcher.io));
}

void UIContentBrowser::StopWatcher()
{
	if (!m_Watcher) return;
	CloseWatcherHandles();
	m_Watcher->root.clear();
	m_Watcher->source = -1;
	m_Watcher->retry_after = 0;
	m_Watcher->last_change = 0;
	m_Watcher->refresh_pending = false;
}

bool UIContentBrowser::ArmWatcher()
{
	if (!m_Watcher) return false;
	SDirectoryWatcher& watcher = *m_Watcher;
	if (watcher.directory == INVALID_HANDLE_VALUE || !watcher.event) return false;

	::ResetEvent(watcher.event);
	ZeroMemory(&watcher.io, sizeof(watcher.io));
	watcher.io.hEvent = watcher.event;
	watcher.armed = !!::ReadDirectoryChangesW(
		watcher.directory, watcher.buffer.data(), DWORD(watcher.buffer.size()), TRUE,
		FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME |
		FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE,
		0, &watcher.io, 0);
	return watcher.armed;
}

bool UIContentBrowser::WatchPathIsRelevant(LPCSTR relative_path) const
{
	if (!m_Watcher || !relative_path || !relative_path[0]) return true;

	char first[MAX_PATH];
	LPCSTR separator = strpbrk(relative_path, "\\/");
	const size_t length = separator ? size_t(separator - relative_path) : xr_strlen(relative_path);
	if (!length || length >= sizeof(first)) return true;
	CopyMemory(first, relative_path, length);
	first[length] = 0;

	if (m_Watcher->source == 0)
	{
		if (EditorProject::IsEditorOnlyEntry(first))	return false;
		// tooling at any depth: git rewrites .git on every status and commit, and
		// each of those would re-list the whole project
		for (LPCSTR seg = relative_path; seg && *seg;)
		{
			LPCSTR sep = strpbrk(seg, "\\/");
			const size_t n = sep ? size_t(sep - seg) : xr_strlen(seg);
			if (n && n < sizeof(first))
			{
				CopyMemory(first, seg, n);
				first[n] = 0;
				if (EditorProject::IsToolingEntry(first))	return false;
			}
			seg = sep ? sep + 1 : 0;
		}
		return true;
	}
	if (m_Watcher->source == 2)
		return 0 == _stricmp(first, "database") || 0 == _stricmp(first, "gamedata");
	return false;
}

void UIContentBrowser::UpdateWatcher()
{
	if (!m_Watcher) return;
	SDirectoryWatcher& watcher = *m_Watcher;

	LPCSTR desired_root = "";
	if (m_Source == 0 && EditorProject::Active())
		desired_root = EditorProject::Root();
	else if (m_Source == 2 && EditorProject::Active() && EditorProject::GameLinked())
		desired_root = EditorProject::GameRoot();

	if (watcher.source != m_Source || 0 != _stricmp(watcher.root.c_str(), desired_root))
	{
		StopWatcher();
		watcher.source = m_Source;
		watcher.root = desired_root;
	}
	if (watcher.root.empty()) return;

	const ULONGLONG now = ::GetTickCount64();
	if (watcher.directory == INVALID_HANDLE_VALUE)
	{
		if (now < watcher.retry_after) return;
		watcher.directory = ::CreateFileA(watcher.root.c_str(), FILE_LIST_DIRECTORY,
			FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, 0, OPEN_EXISTING,
			FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED, 0);
		watcher.event = ::CreateEventA(0, TRUE, FALSE, 0);
		if (watcher.directory == INVALID_HANDLE_VALUE || !watcher.event || !ArmWatcher())
		{
			CloseWatcherHandles();
			watcher.retry_after = now + 2000;
			return;
		}
	}

	const DWORD wait = ::WaitForSingleObject(watcher.event, 0);
	if (wait == WAIT_OBJECT_0)
	{
		DWORD bytes = 0;
		bool relevant = false;
		const bool completed = !!::GetOverlappedResult(watcher.directory, &watcher.io, &bytes, FALSE);
		watcher.armed = false;
		if (!completed || !bytes)
		{
			// Buffer overflow and watcher failures both require one conservative rescan.
			relevant = true;
		}
		else
		{
			DWORD offset = 0;
			while (offset < bytes)
			{
				const DWORD header_size = FIELD_OFFSET(FILE_NOTIFY_INFORMATION, FileName);
				if (bytes - offset < header_size)
				{
					relevant = true;
					break;
				}
				const FILE_NOTIFY_INFORMATION* info =
					reinterpret_cast<const FILE_NOTIFY_INFORMATION*>(watcher.buffer.data() + offset);
				if ((info->FileNameLength % sizeof(wchar_t)) ||
					info->FileNameLength > bytes - offset - header_size)
				{
					relevant = true;
					break;
				}

				const int wide_count = int(info->FileNameLength / sizeof(wchar_t));
				int first_count = 0;
				while (first_count < wide_count && info->FileName[first_count] != L'\\' &&
					info->FileName[first_count] != L'/')
				{
					++first_count;
				}
				char relative[MAX_PATH];
				const int converted = first_count > 0 && first_count < int(sizeof(relative))
					? ::WideCharToMultiByte(CP_ACP, 0, info->FileName, first_count,
						relative, int(sizeof(relative)) - 1, 0, 0)
					: 0;
				if (converted > 0)
				{
					relative[converted] = 0;
					relevant = relevant || WatchPathIsRelevant(relative);
				}
				else
					relevant = true;

				if (!info->NextEntryOffset) break;
				if (info->NextEntryOffset < header_size || info->NextEntryOffset > bytes - offset)
				{
					relevant = true;
					break;
				}
				offset += info->NextEntryOffset;
			}
		}

		if (!completed || !ArmWatcher())
		{
			CloseWatcherHandles();
			watcher.retry_after = now + 2000;
		}
		if (relevant)
		{
			watcher.last_change = now;
			watcher.refresh_pending = true;
		}
	}
	else if (wait == WAIT_FAILED)
	{
		CloseWatcherHandles();
		watcher.retry_after = now + 2000;
		watcher.last_change = now;
		watcher.refresh_pending = true;
	}

	// Only refresh after the writer goes quiet, so long exports cannot remount in flight.
	if (watcher.refresh_pending && now - watcher.last_change >= 300)
	{
		if (watcher.source == 2) EditorGameContent::Unmount();
		m_NeedRefresh = true;
		watcher.refresh_pending = false;
		watcher.last_change = 0;
	}
}

void UIContentBrowser::EnsureListing()
{
	SyncFavoriteScopes();
	LPCSTR project_root = EditorProject::Active() ? EditorProject::Root() : "";
	LPCSTR game_root = EditorProject::GameRoot();
	const bool project_changed = 0 != _stricmp(m_ListedProject.c_str(), project_root);
	const bool game_changed = 0 != _stricmp(m_ListedGameRoot.c_str(), game_root);
	const bool invalidate_project = project_changed;
	const bool invalidate_darf = project_changed || game_changed;

	if (project_changed || game_changed)
	{
		m_ListedProject = project_root;
		m_ListedGameRoot = game_root;
		RemoveScopedLocations(m_BackHistory, invalidate_project, invalidate_darf);
		RemoveScopedLocations(m_ForwardHistory, invalidate_project, invalidate_darf);

		if (invalidate_darf)
		{
			m_DarfReady = false;
			m_DarfStatus = "Reading the linked game.";
		}

		if ((m_Source == 0 && invalidate_project) || (m_Source == 2 && invalidate_darf))
		{
			m_NeedRefresh = true;
			m_CurFolder.clear();
			ClearSelection();
			m_Rename.clear();
			m_RenameFocus = false;
		}
	}
	if (m_NeedRefresh) Refresh();
}

void UIContentBrowser::Refresh()
{
	m_NeedRefresh	= false;
	InvalidateView	();
	// what an .nqasset or an .npcasset IS was cached for the icons; the files may
	// have changed, and the tiles must not ask the disk once per frame to find out
	EditorAssetIcons::Invalidate();
	// ... and who the project's characters are was cached for the quest validator,
	// and what the build leaves out for the tiles (mod.ltx may have changed on disk)
	EditorCharacters::Invalidate();
	EditorMod::InvalidateExclusions();
	// the preview's textures: the project's own sheets may have changed
	EditorPda::Invalidate();
	m_Items.clear	();
	m_Dirs.clear	();
	DropCache		();

	if (m_Source == 0)
	{
		// The project ROOT is the content root. A module may keep any layout it
		// likes - mod.ltx [vfs] publishes it wherever the game needs it - so
		// every folder the author makes here is content by definition. Listing
		// only gamedata and rawdata hid the module's own parts (patch, spawn,
		// scripts, levels) and made the root look like a place you cannot work.
		if (EditorProject::Active())
		{
			WIN32_FIND_DATAA fd;
			string_path mask;
			HANDLE h = INVALID_HANDLE_VALUE;
			if (xr_strlen(EditorProject::Root()) + 2 < sizeof(mask))
			{
				sprintf_s(mask, "%s\\*", EditorProject::Root());
				h = ::FindFirstFileA(mask, &fd);
			}
			if (INVALID_HANDLE_VALUE != h)
			{
				do
				{
					if (!strcmp(fd.cFileName, ".") || !strcmp(fd.cFileName, "..")) continue;
					// the editor's own scratch is not the author's content, and tooling
					// (.git, .gitignore, Thumbs.db, ...) is nobody's
					if (EditorProject::IsEditorOnlyEntry(fd.cFileName)) continue;
					if (EditorProject::IsToolingEntry(fd.cFileName)) continue;
					if (!ContentAbsolutePathFits(EditorProject::Root(), xr_strlen(fd.cFileName))) continue;
					if (fd.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) continue;
					if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)
					{
						m_Dirs.push_back(xr_string(fd.cFileName));
						ScanContentDir(EditorProject::Root(), fd.cFileName, m_Items, m_Dirs);
					}
					else
						m_Items.push_back(SChooseItem(fd.cFileName, ""));
				} while (::FindNextFileA(h, &fd));
				::FindClose(h);
			}

			// The module's own machinery - the manifest and what export
			// generates - is not content and must not be deletable by a stray
			// Ctrl+A + Del. It is edited through Mod Settings, and the
			// generated files follow the manifest on their own.
			xr_vector<xr_string> owned;
			EditorMod::GetOwnedFiles(EditorProject::Root(), owned);
			for (u32 k = 0; k < m_Items.size();)
			{
				bool hide = false;
				for (u32 o = 0; o < owned.size() && !hide; ++o)
					hide = (0 == _stricmp(m_Items[k].name.c_str(), owned[o].c_str()));
				if (hide)	m_Items.erase(m_Items.begin() + k);
				else		++k;
			}
		}
		m_Root.name = "Project";
	}
	else if (m_Source == 2)
	{
		// linked game install: archives + loose gamedata as one virtual tree
		m_Root.name = "Game";
		xr_string err;
		m_DarfReady = EditorGameContent::EnsureMounted(err);
		if (m_DarfReady)
		{
			m_DarfStatus = "";
			const int n = EditorGameContent::Count();
			m_Items.reserve(n);
			for (int i = 0; i < n; ++i)
				m_Items.push_back(SChooseItem(EditorGameContent::Get(i)->path.c_str(), ""));
		}
		else
		{
			m_DarfStatus  = "The linked game could not be read: ";
			m_DarfStatus += err;
		}
	}
	else if (kCategories[m_Category].id == kLevelsCategoryId)
	{
		// Levels are editor scenes, not library assets - they have no entry in
		// the choose-event table, so they are listed straight off the maps root.
		//
		// TWO sources, and the second one is the point: opening a project
		// remounts $maps$ onto that project, so the scenes the editor ships -
		// the sources every stock level was built from - vanish from the FS the
		// moment there is a project. This category is Editor Content: it lists
		// the library, whatever the project has mounted over it.
		m_Root.name = "Editor Content";
		FS_FileSet lst;
		if (FS.file_list(lst, _maps_, FS_ListFiles | FS_ClampExt, "*.level"))
		{
			FS_FileSetIt it = lst.begin(), e = lst.end();
			for (; it != e; ++it)
				m_Items.push_back(SChooseItem(it->name.c_str(), ""));
		}
		xr_vector<xr_string> base;
		EditorProject::ListBaseScenes(base);
		for (u32 i = 0; i < base.size(); ++i)
		{
			bool have = false;
			for (u32 k = 0; k < m_Items.size() && !have; ++k)
				have = (0 == _stricmp(m_Items[k].name.c_str(), base[i].c_str()));
			if (!have) m_Items.push_back(SChooseItem(base[i].c_str(), ""));
		}
	}
	else
	{
		SChooseEvents* E = UIChooseForm::GetEvents(kCategories[m_Category].id);
		if (E && E->on_fill) E->on_fill(m_Items, 0);
		m_Root.name = "Editor Content";
	}

	BuildTree		();
	if (!FindFolder(m_CurFolder.c_str()))
	{
		m_CurFolder.clear();
		ClearSelection();
	}
	PruneSelection();
	if (m_Watcher && m_Watcher->refresh_pending)
	{
		m_Watcher->refresh_pending = false;
		m_Watcher->last_change = 0;
	}
}

void UIContentBrowser::PruneSelection()
{
	xr_flat_hash_map<SEntry, u8, SEntryHash> available;
	available.reserve(m_Items.size() + m_Dirs.size() + 1);
	for (u32 i = 0; i < m_Items.size(); ++i)
		available.emplace(SEntry(m_Items[i].name.c_str(), false), u8(1));

	xr_vector<SFolder*> stack;
	stack.push_back(&m_Root);
	while (!stack.empty())
	{
		SFolder* folder = stack.back();
		stack.pop_back();
		for (u32 i = 0; i < folder->children.size(); ++i)
		{
			SFolder& child = folder->children[i];
			available.emplace(SEntry(child.path.c_str(), true), u8(1));
			stack.push_back(&child);
		}
	}

	for (u32 i = 0; i < m_Selection.size();)
	{
		if (available.find(m_Selection[i]) == available.end())
			m_Selection.erase(m_Selection.begin() + i);
		else
			++i;
	}
	RebuildSelectionLookup();
	if (m_HasAnchor && m_SelectionLookup.find(m_Anchor) == m_SelectionLookup.end())
	{
		m_HasAnchor = !m_Selection.empty();
		if (m_HasAnchor) m_Anchor = m_Selection.back();
	}
	if (m_HasPendingRange && available.find(m_PendingRange) == available.end())
		m_HasPendingRange = false;
	if (m_Selection.empty()) m_ScrollToSelection = false;
	if (!m_Rename.empty() &&
		available.find(SEntry(m_Rename.c_str(), false)) == available.end() &&
		available.find(SEntry(m_Rename.c_str(), true)) == available.end())
	{
		m_Rename.clear();
		m_RenameFocus = false;
	}
}

void UIContentBrowser::SwitchSource(int src)
{
	if (m_Source == src) return;
	xr_string err;
	ApplyLocation(SLocation(src, int(m_Category), ""), true, err);
}

UIContentBrowser::SFolder* UIContentBrowser::EnsureFolder(LPCSTR path)
{
	SFolder* cur = &m_Root;
	if (!path || !path[0]) return cur;

	xr_string acc;
	LPCSTR p = path;
	for (;;)
	{
		LPCSTR slash = strchr(p, '\\');
		// the tail after the last separator is a folder too when it came from
		// the directory list; BuildTree stops one short for item names
		const int len = slash ? int(slash - p) : int(xr_strlen(p));
		if (!len) break;

		string_path buf;
		int n = len;
		if (n > (int)sizeof(buf) - 1) n = sizeof(buf) - 1;
		CopyMemory(buf, p, n);
		buf[n] = 0;

		xr_string part(buf);
		if (!acc.empty()) acc += "\\";
		acc += part;

		SFolder* next = 0;
		for (u32 c = 0; c < cur->children.size(); ++c)
			if (cur->children[c].name == part) { next = &cur->children[c]; break; }

		if (!next)
		{
			SFolder f; f.name = part; f.path = acc;
			cur->children.push_back(f);
			next = &cur->children.back();
		}
		cur = next;
		if (!slash) break;
		p = slash + 1;
	}
	return cur;
}

void UIContentBrowser::BuildTree()
{
	m_Root.children.clear();
	m_Root.items.clear();

	// folders that exist on disk first, so an empty one still shows up
	for (u32 d = 0; d < m_Dirs.size(); ++d)
		EnsureFolder(m_Dirs[d].c_str());

	for (int i = 0; i < (int)m_Items.size(); ++i)
	{
		LPCSTR nm = m_Items[i].name.c_str();
		if (!nm || !nm[0]) continue;

		// everything up to the last separator is folders, the tail is the item
		LPCSTR slash = strrchr(nm, '\\');
		SFolder* cur = &m_Root;
		if (slash)
		{
			string_path dir;
			int len = int(slash - nm);
			if (len > (int)sizeof(dir) - 1) len = sizeof(dir) - 1;
			CopyMemory(dir, nm, len);
			dir[len] = 0;
			cur = EnsureFolder(dir);
		}
		cur->items.push_back(i);
	}
}

void UIContentBrowser::CollectItems(SFolder& f, xr_vector<int>& out, bool recursive)
{
	for (u32 i = 0; i < f.items.size(); ++i) out.push_back(f.items[i]);
	if (recursive)
		for (u32 c = 0; c < f.children.size(); ++c) CollectItems(f.children[c], out, true);
}

// Queues a model render for `name`, picking the right source: DARF items are
// only in the private archive VFS, project items only on disk, library visuals
// are resolvable by name.
void UIContentBrowser::RequestModelThumbnail(LPCSTR name)
{
	if (HasVisualThumbnailRequest(name)) return;

	if (m_Source == 2)
	{
		const int idx = EditorGameContent::Find(name);
		u32 sz = 0;
		u8* bytes = (idx >= 0) ? EditorGameContent::ReadBytes(idx, sz) : 0;
		if (bytes && sz)
			QueueVisualThumbnailFromMemory(name, bytes, sz);
		else
		{
			// not readable from the archives - the editor ships its own copy of
			// the mesh tree, so try the same path there
			char visual[MAX_PATH];
			ToVisualName(name, visual, sizeof(visual));
			QueueVisualThumbnail(name, visual);
		}
		EditorGameContent::FreeBytes(bytes);
	}
	else if (m_Source == 0)
	{
		xr_vector<u8> bytes;
		if (ReadProjectFile(name, bytes))
		{
			// .object is the editor's source format and .ogf the engine's; both
			// render offscreen, but through different loaders
			if (IsObjectExt(name))	QueueObjectThumbnailFromMemory(name, bytes.data(), u32(bytes.size()));
			else					QueueVisualThumbnailFromMemory(name, bytes.data(), u32(bytes.size()));
		}
	}
	else
	{
		QueueVisualThumbnail(name, name);
	}
}

// width / height of a thumbnail: a picture that is not square keeps its shape in the tile
static float TextureAspect(ImTextureID tex)
{
	float aspect = 1.f;
#if defined(USE_DX11)
	ID3D11Resource* resource = nullptr;
	if (tex)
		tex->GetResource(&resource);
	if (resource)
	{
		ID3D11Texture2D* texture = nullptr;
		if (SUCCEEDED(resource->QueryInterface(__uuidof(ID3D11Texture2D), (void**)&texture)) && texture)
		{
			D3D11_TEXTURE2D_DESC desc;
			texture->GetDesc(&desc);
			if (desc.Width && desc.Height)
				aspect = float(desc.Width) / float(desc.Height);
			texture->Release();
		}
		resource->Release();
	}
#endif
	return aspect;
}

// Pictures are decoded inline - one is quick, and a model render goes through its queue anyway - but never more than
// the frame's budget: a folder of large textures fills over a few frames.
bool UIContentBrowser::TakeDecodeTurn()
{
	if (!m_ThumbsThisFrame)
		m_ThumbClock.Start();
	else if (m_ThumbClock.GetElapsed_ms() >= kThumbDecodeMs)
		return false;
	++m_ThumbsThisFrame;
	return true;
}

bool UIContentBrowser::MakeImageThumb(LPCSTR name, u32 side, SThumb& t)
{
	t = SThumb();
	t.last_used = m_Tick;
#if defined(USE_DX11)
	STextureTile tile;
	bool made = false;
	if (IsGameSource())
	{
		// archive-backed files have no disk path: the bytes come through the private VFS
		const int idx = EditorGameContent::Find(name);
		u32 sz = 0;
		u8* bytes = (idx >= 0) ? EditorGameContent::ReadBytes(idx, sz) : 0;
		made = bytes && sz && TextureTileFromMemory(bytes, sz, side, tile);
		EditorGameContent::FreeBytes(bytes);
	}
	else if (EditorProject::Active())
	{
		const xr_string abs = xr_string(EditorProject::Root()) + "\\" + name;
		made = TextureTileFromFile(abs.c_str(), side, tile);
	}
	if (made)
		t.tex = DX11TextureFromPixels(tile.pixels.data(), tile.w, tile.h);
	if (!t.tex)
		return false;
	t.aspect	= float(tile.w) / float(tile.h);
	t.side		= side;
	t.edge		= _max(tile.w, tile.h);
	t.full		= t.edge >= _max(tile.src_w, tile.src_h);
	t.alpha		= tile.alpha;
	t.src_w		= tile.src_w;
	t.src_h		= tile.src_h;
	return true;
#else
	IDirect3DTexture9* tex = 0;
	if (IsGameSource())
	{
		const int idx = EditorGameContent::Find(name);
		u32 sz = 0;
		u8* bytes = (idx >= 0) ? EditorGameContent::ReadBytes(idx, sz) : 0;
		if (bytes && sz)
			D3DXCreateTextureFromFileInMemoryEx(HW.pDevice, bytes, sz, D3DX_DEFAULT, D3DX_DEFAULT, 1, 0, D3DFMT_X8R8G8B8,
				D3DPOOL_MANAGED, D3DX_FILTER_LINEAR, D3DX_FILTER_NONE, 0, NULL, NULL, &tex);
		EditorGameContent::FreeBytes(bytes);
	}
	else if (EditorProject::Active())
	{
		const xr_string abs = xr_string(EditorProject::Root()) + "\\" + name;
		D3DXCreateTextureFromFileExA(HW.pDevice, abs.c_str(), D3DX_DEFAULT, D3DX_DEFAULT, 1, 0, D3DFMT_X8R8G8B8,
			D3DPOOL_MANAGED, D3DX_FILTER_LINEAR, D3DX_FILTER_NONE, 0, NULL, NULL, &tex);
	}
	t.tex = tex;
	t.aspect = TextureAspect(t.tex);
	return !!t.tex;
#endif
}

// Evict the coldest entry once the budget is blown - but NEVER one touched this frame. A tile drawn this frame has its
// view recorded in an ImGui draw list that has not been rendered yet; releasing it here hands the driver a freed
// pointer at render time, which is exactly the flaky igd10um64xe crash a folder bigger than the budget (prop: 759
// textures vs 512) produced. Strict '<' against m_Tick is the whole fix: a folder that outgrows the budget simply
// keeps its tiles cached while it is on screen.
const UIContentBrowser::SThumb* UIContentBrowser::StoreThumb(const shared_str& key, const SThumb& t)
{
	if (m_Thumbs.size() >= kThumbBudget)
	{
		u32 oldest = m_Tick;
		ThumbMapIt victim = m_Thumbs.end();
		for (ThumbMapIt i = m_Thumbs.begin(); i != m_Thumbs.end(); ++i)
			if (i->second.last_used < oldest) { oldest = i->second.last_used; victim = i; }
		if (victim != m_Thumbs.end())
		{
			ReleaseThumbTexture(victim->second.tex);
			m_Thumbs.erase(victim);
		}
	}
	return &m_Thumbs.insert(mk_pair(key, t)).first->second;
}

const UIContentBrowser::SThumb* UIContentBrowser::GetThumb(LPCSTR name, float px)
{
	shared_str key(name);
	// What an image's picture is made for: the power of two at or over the px it is drawn at, so it is never shrunk
	// to half or less - ImGui samples one level, and at 1:20 a 2048 px atlas loses its thin lines.
	u32 want = 32;
	while (want < 512 && float(want) < px)
		want <<= 1;

	ThumbMapIt it = m_Thumbs.find(key);
	if (it != m_Thumbs.end())
	{
		SThumb& cached = it->second;
		// Drawn at another size, an image gets a picture made for it once the frame has the time; the old one shows
		// meanwhile. Never one drawn this frame already: its view waits in a draw list not rendered yet.
		const bool remake = cached.side && cached.last_used != m_Tick &&
			((want > cached.side && !cached.full) || (want < cached.side && cached.edge > want));
		cached.last_used = m_Tick;
		if (remake && TakeDecodeTurn())
		{
			SThumb made;
			if (MakeImageThumb(name, want, made))
			{
				ReleaseThumbTexture(cached.tex);
				cached = made;
			}
			else
				cached.side = 0;	// the file went meanwhile: the picture there is stays, and nobody asks again
		}
		return &cached;
	}

	SThumb t;
	t.last_used = m_Tick;

	// Models are never rendered here: the request goes into a queue that the
	// frame loop drains outside the scene (see EThumbnailVisual.h). Rendering
	// inline would mean closing the editor's scene from inside its own ImGui
	// pass, and any throw in between takes the next frame down with it.
	// .object counts here too: in the project source it is the usual way a model
	// arrives, and it has no baked .thm beside it the way the SDK library does
	const bool needs_render = IsVisualExt(name) || (m_Source == 0 && IsObjectExt(name));
	const bool will_render	= needs_render ||
							  (m_Source == 1 && kCategories[m_Category].id == smVisual);
	if (will_render)
	{
		U32Vec	pixels;
		bool	failed = false;
		if (!TakeVisualThumbnail(name, pixels, failed))
		{
			RequestModelThumbnail(name);
			return nullptr;	// not cached: the tile retries until the queue answers
		}
		// a failed render caches a null texture, so it is not asked for again
		t.tex = failed ? 0 : static_cast<ImTextureID>(XFinedMCP::PixelsToTexture(pixels));
		t.aspect = TextureAspect(t.tex);
		return StoreThumb(key, t);
	}

	// An image of the project or the game gets its picture; any other file caches a null one and the tile draws its
	// glyph. The game tree dwarfs the SDK one and a UI atlas is 2048 px: the frame's budget spreads the decoding.
	if (1 != m_Source)
	{
		if (IsImageExt(name))
		{
			if (!TakeDecodeTurn())
				return nullptr;
			MakeImageThumb(name, want, t);
		}
		return StoreThumb(key, t);
	}

	const bool object = kCategories[m_Category].id == smObject;
	bool object_result = false;
	ImTextureID tex = 0;
	if (object)
	{
		U32Vec pixels;
		bool failed = false;
		if (TakeVisualThumbnail(name, pixels, failed))
		{
			tex = failed ? 0 : static_cast<ImTextureID>(XFinedMCP::PixelsToTexture(pixels));
			object_result = true;
		}
		else if (HasVisualThumbnailRequest(name))
			return nullptr;
	}

	if (!object_result)
	{
		SChooseEvents* E = UIChooseForm::GetEvents(kCategories[m_Category].id);
		if (E && E->on_get_texture) E->on_get_texture(name, tex);

		// Roughly half of the shipped .object files have no baked .thm, so their
		// tiles stayed blank. Render those instead of giving up.
		if (!tex && object)
		{
			QueueObjectThumbnail(name, name);
			return nullptr;	// retry next frame, do not cache a miss
		}
	}
	t.tex = tex;
	t.aspect = TextureAspect(tex);
	return StoreThumb(key, t);
}

//------------------------------------------------------------------------------
// ui
//------------------------------------------------------------------------------
// Unreal's scroll-into-view rules, used by both panes here.
//
// The first one is the one that matters: something already on screen is left
// EXACTLY where it is (SListView::ScrollIntoView - "Only scroll the item into
// view if it's not already in the visible range"). Recentring a tile the user
// can already see is what makes a browser feel like it ran away from the
// cursor. Past that, STileView centres a target that is far off, and merely
// pulls in one that is a row or so past an edge, leaving half a row of the
// next one visible (its NavigationScrollOffset, 0.5).
static void ScrollRectIntoView(float top_screen, float bottom_screen)
{
	const float view_top	= ImGui::GetScrollY();
	const float view_h		= ImGui::GetWindowHeight();
	const float win_y		= ImGui::GetWindowPos().y;
	// screen -> content space, which is what GetScrollY/SetScrollY speak
	const float top			= top_screen    - win_y + view_top;
	const float bottom		= bottom_screen - win_y + view_top;
	const float h			= bottom - top;

	if (top >= view_top && bottom <= view_top + view_h)	return;		// on screen

	if (top < view_top && top >= view_top - 2.f * h)
		ImGui::SetScrollY(top - h * 0.5f);					// just above
	else if (bottom > view_top + view_h && bottom <= view_top + view_h + 2.f * h)
		ImGui::SetScrollY(bottom - view_h + h * 0.5f);		// just below
	else
		ImGui::SetScrollY(top + h * 0.5f - view_h * 0.5f);	// far away: centre
}

// "rawdata" holds "rawdata\levels"; the root's empty path holds everything
static bool PathIsAncestor(const xr_string& parent, const xr_string& path)
{
	if (parent.empty())					return true;
	if (parent.size() > path.size())	return false;
	if (0 != _strnicmp(parent.c_str(), path.c_str(), parent.size()))	return false;
	return parent.size() == path.size() || path[parent.size()] == '\\';
}

void UIContentBrowser::DrawFolder(SFolder& f)
{
	const bool root	= &f == &m_Root;
	const bool leaf	= f.children.empty();
	int flags		= ImGuiTreeNodeFlags_OpenOnArrow | ImGuiTreeNodeFlags_OpenOnDoubleClick |
					  ImGuiTreeNodeFlags_SpanAvailWidth | ImGuiTreeNodeFlags_FramePadding;
	if (leaf)					flags |= ImGuiTreeNodeFlags_Leaf | ImGuiTreeNodeFlags_NoTreePushOnOpen;
	if (m_CurFolder == f.path)	flags |= ImGuiTreeNodeFlags_Selected;
	if (root)					flags |= ImGuiTreeNodeFlags_DefaultOpen;

	// a reveal drops the browser into a folder this tree may have collapsed -
	// open the chain down to it, so the pane agrees with the grid beside it
	if (m_ScrollToSelection && !leaf && PathIsAncestor(f.path, m_CurFolder))
		ImGui::SetNextItemOpen(true, ImGuiCond_Always);

	// a folder the build leaves out reads as such here too
	const EditorMod::EShipState ship = (m_Source == 0 && EditorProject::Active() && !f.path.empty())
		? EditorMod::ShipState(EditorProject::Root(), f.path.c_str()) : EditorMod::shipShipped;
	const bool veiled = ship == EditorMod::shipExcluded || ship == EditorMod::shipExcludedByRule;

	// the node carries no label: the icon and a name cut to the pane follow it on its line
	const xr_string id = "##" + f.path;
	const bool open = ImGui::TreeNodeEx(root ? "##root" : id.c_str(), flags);
	const ImVec2 row_min = ImGui::GetItemRectMin(), row_max = ImGui::GetItemRectMax();
	if (ImGui::IsItemClicked(ImGuiMouseButton_Left) && !ImGui::IsItemToggledOpen())
	{
		xr_string err;
		ApplyLocation(SLocation(m_Source, int(m_Category), f.path.c_str()), true, err);
		// A folder is an entry like any other: clicking it in the tree selects
		// it, so Delete / Copy / Rename act on it exactly as they do on a tile.
		// The root has no path of its own and cannot be one of them.
		if (!root)	SelectEntry(f.path.c_str(), true, false, false);
		else		ClearSelection();
	}
	// the SAME menu the tiles get - a folder does not become a different thing
	// because it is drawn on the left; its id is the folder's, not the tree level's
	if (!root)
	{
		ImGui::PushID(f.path.c_str());
		DrawEntryContextMenu(f.path.c_str(), true);
		ImGui::PopID();
	}
	if (ImGui::IsItemHovered() && veiled)
		XUI::Tip("Not shipped: mod.ltx [exclude] leaves it out of the build.");

	ImGui::SameLine(0.f, 0.f);
	const ImVec2 at = ImGui::GetCursorScreenPos();
	const float icon = XUI::Pt(14.f);
	XUI::DrawIcon(ImGui::GetWindowDrawList(), open && !leaf ? XUI::Icon::FolderOpen : XUI::Icon::Folder,
		ImVec2(PxRound(at.x), PxRound((row_min.y + row_max.y - icon) * .5f)), 14.f,
		ThemeCol(veiled ? Token::TextDisabled : Token::TextSecondary));
	ImGui::SetCursorScreenPos(ImVec2(at.x + icon + XUI::Pt(6.f), at.y));
	if (veiled)	ImGui::PushStyleColor(ImGuiCol_Text, ThemeCol(Token::TextMuted));
	XUI::TextEllipsis(root ? RootLabel() : f.name.c_str());
	if (veiled)	ImGui::PopStyleColor();

	if (m_ScrollToSelection && f.path == m_CurFolder)
		ScrollRectIntoView(row_min.y, row_max.y);

	if (open && !leaf)
	{
		for (u32 i = 0; i < f.children.size(); ++i) DrawFolder(f.children[i]);
		ImGui::TreePop();
	}
}

// Turns whatever the Levels listing calls a level into a file COMMAND_LOAD can
// actually open. The listing clamps the extension off, and a level may be a
// bare "<name>.level" or a folder holding "<name>\<name>.level", so all the
// shapes are tried before giving up.
bool UIContentBrowser::ResolveLevelFile(LPCSTR name, int source, string_path& out)
{
	out[0] = 0;
	if (!name || !name[0])	return false;

	// the project's own Content names files relative to the project root
	if (0 == source && EditorProject::Active())
	{
		string_path abs;
		xr_sprintf	(abs, sizeof(abs), "%s\\%s", EditorProject::Root(), name);
		if (INVALID_FILE_ATTRIBUTES != ::GetFileAttributesA(abs))
		{
			xr_strcpy(out, sizeof(out), abs);
			return true;
		}
	}

	string_path stem;
	xr_strcpy	(stem, sizeof(stem), name);
	if (char* dot = strrchr(stem, '.'))
		if (!strchr(dot, '\\')) *dot = 0;

	LPCSTR leaf = strrchr(stem, '\\');
	leaf = leaf ? leaf + 1 : stem;

	string_path rel;
	// as listed, plus the extension
	xr_sprintf	(rel, sizeof(rel), "%s.level", stem);
	if (FS.exist(out, _maps_, rel))						return true;
	// a level folder: <stem>\<leaf>.level
	xr_sprintf	(rel, sizeof(rel), "%s\\%s.level", stem, leaf);
	if (FS.exist(out, _maps_, rel))						return true;

	// The game's own layout: levels\<name>\level.* is a COMPILED level - there
	// is no scene in there to open, the editor works on the source it was built
	// from. Map it to that source by name, which is what the author means by
	// "open l05_bar" while looking at the game's content.
	if (0 == _strnicmp(name, "levels\\", 7) || 0 == _strnicmp(name, "levels/", 7))
	{
		string_path lvl;
		xr_strcpy(lvl, sizeof(lvl), name + 7);
		for (char* p = lvl; *p; ++p)
			if (*p == '/') *p = '\\';
		if (char* cut = strchr(lvl, '\\')) *cut = 0;
		if (lvl[0])
		{
			xr_sprintf(rel, sizeof(rel), "%s.level", lvl);
			if (FS.exist(out, _maps_, rel))				return true;
			xr_sprintf(rel, sizeof(rel), "%s\\%s.level", lvl, lvl);
			if (FS.exist(out, _maps_, rel))				return true;
		}
	}

	// already a full path
	if (FS.exist(name))	{ xr_strcpy(out, sizeof(out), name); return true; }

	return false;
}

// Is this listing entry one of the game's levels? Everything under levels\ in
// the game source belongs to one, folder and compiled file alike.
bool UIContentBrowser::IsGameLevelEntry(LPCSTR name) const
{
	if (2 != m_Source || !name)	return false;
	if (0 != _strnicmp(name, "levels\\", 7) && 0 != _strnicmp(name, "levels/", 7)) return false;
	return name[7] != 0;
}

// Double-click action, Unreal-style: open the asset, never modify the scene.
// Models get the preview window; kinds with a dedicated editor go there; the
// rest report that they have no viewer yet instead of doing something random.
// `err` non-null means a caller wants the outcome back instead of a dialog -
// MCP drives this too, and a modal there would hang the editor on a machine
// nobody is sitting at.
bool UIContentBrowser::OpenAsset(LPCSTR name, xr_string* err)
{
#define OPEN_FAILED(...)	do {											\
		string256 _m; xr_sprintf(_m, sizeof(_m), __VA_ARGS__);			\
		if (err) *err = _m; else ELog.DlgMsg(mtError, "%s", _m);			\
		return false; } while(0)

	if (err) err->clear();
	if (!name || !name[0])	OPEN_FAILED("empty asset name");

	// A level IS the scene: opening one loads it, which is the one case where
	// the double-click legitimately changes what the editor is working on.
	// COMMAND_LOAD asks about unsaved changes itself.
	//
	// Both by extension and by category: the library lists levels by name with
	// the extension clamped off, while the project's own Content shows the
	// .level FILE - and a double click there has to do the same thing.
	const bool is_level = IsLevelExt(name) ||
						  (m_Source == 1 && kCategories[m_Category].id == kLevelsCategoryId) ||
						  IsGameLevelEntry(name);
	if (is_level)
	{
		// COMMAND_LOAD feeds its argument straight to FS.r_open and returns FALSE
		// without a word when that fails - and the listing hands out names with
		// the extension clamped off, which never opens. Resolve to a real file
		// first, and say so when there is none.
		string_path fn;
		if (!ResolveLevelFile(name, m_Source, fn))
		{
			// Not in the project, so it is the editor's own library copy: bring
			// it into the project and open THAT. The author edits their own
			// scene from the first click, and the shared library stays clean -
			// the same thing File > Import Base Scene does, without the detour.
			string_path scene;
			xr_strcpy(scene, sizeof(scene), name);
			for (char* p = scene; *p; ++p) if (*p == '/') *p = '\\';
			if (char* dot = strrchr(scene, '.'))
				if (!strchr(dot, '\\')) *dot = 0;
			// the game names a level by its folder: levels\l05_bar\...
			if (IsGameLevelEntry(scene))
			{
				xr_string tail = scene + 7;
				const size_t cut = tail.find('\\');
				xr_strcpy(scene, sizeof(scene), (cut == xr_string::npos ? tail : tail.substr(0, cut)).c_str());
			}

			xr_string import_err;
			if (EditorProject::HasBaseScene(scene) &&
				EditorProject::ImportBaseScene(scene, fn, import_err))
			{
				ELog.Msg(mtInformation, "Scene '%s' copied into the project - you are editing your own copy.", scene);
			}
			else if (IsGameLevelEntry(name))
			{
				// The game's levels are compiled: what the editor opens is the
				// source scene they were built from, and an install can ship a
				// level nobody has the source for.
				OPEN_FAILED("'%s' is a COMPILED level. The editor opens the SOURCE scene it was built\n"
							"from, and neither this project nor the editor library has one under that\n"
							"name.%s%s", name,
							import_err.empty() ? "" : "\n", import_err.c_str());
			}
			else
				OPEN_FAILED("no .level file behind '%s'", name);
		}

		ExecCommand(COMMAND_LOAD, xr_string(fn));
		return true;
	}

	// Library objects (.object) are previewable too - the preview window loads
	// them through CEditableObject when the model pool has nothing under that
	// name. Without this the most-used category in the browser opened nothing.
	// A project's own .object opens like a library one: it is the most common
	// thing in there, and it used to fall through to "no viewer for this kind".
	const bool is_model = IsVisualExt(name) || IsObjectExt(name) ||
						  (m_Source == 1 && (kCategories[m_Category].id == smVisual ||
											 kCategories[m_Category].id == smObject));
	if (is_model && (s_ShowVisual || s_ShowVisualMem || s_ShowObjectMem))
	{
		if (m_Source == 2 && s_ShowVisualMem)
		{
			const int idx = EditorGameContent::Find(name);
			u32 sz = 0;
			u8* bytes = (idx >= 0) ? EditorGameContent::ReadBytes(idx, sz) : 0;
			if (bytes && sz)		s_ShowVisualMem(name, bytes, sz);
			else if (s_ShowVisual)
			{
				char visual[MAX_PATH];
				ToVisualName(name, visual, sizeof(visual));
				s_ShowVisual(visual);
			}
			EditorGameContent::FreeBytes(bytes);
		}
		else if (m_Source == 0)
		{
			// off the disk: nothing under the project root is in the editor FS
			xr_vector<u8> bytes;
			if (!ReadProjectFile(name, bytes))
				OPEN_FAILED("can't read '%s' from the project folder", name);
			const bool is_object = IsObjectExt(name);
			if (is_object && s_ShowObjectMem)		s_ShowObjectMem(name, bytes.data(), u32(bytes.size()));
			else if (!is_object && s_ShowVisualMem)	s_ShowVisualMem(name, bytes.data(), u32(bytes.size()));
			else									OPEN_FAILED("no viewer registered for '%s'", name);
		}
		else if (s_ShowVisual)
			s_ShowVisual(name);
		return true;
	}

	// A PDA layout opens in its preview tab - project files only: the tab saves its text.
	if (IsPdaExt(name))
	{
		if (m_Source != 0) OPEN_FAILED("%s: PDA layouts open from the project only", name);
		string_path abs;
		sprintf_s(abs, "%s\\%s", EditorProject::Root(), name);
		EditorPda::Open(abs);
		return true;
	}

	// A character opens in the Characters window - project files only, for the
	// same reason a quest does: the window writes the asset back.
	if (IsCharacterExt(name))
	{
		if (m_Source != 0) OPEN_FAILED("%s: characters open from the project only", name);
		string_path abs;
		sprintf_s(abs, "%s\\%s", EditorProject::Root(), name);
		EditorCharacters::Open(abs);
		return true;
	}

	// Quest graphs open in their own tab (project files only: the graph editor
	// writes back through NqDoc, and the game/library sources are read-only)
	if (IsQuestExt(name))
	{
		if (m_Source != 0) OPEN_FAILED("%s: quest graphs open from the project only", name);
		xr_string q_err;
		if (UIQuestGraph::Open(name, &q_err)) return true;
		OPEN_FAILED("%s: %s", name, q_err.c_str());
	}

	// Textures get their own window, whatever source they came from. In the SDK
	// library the name carries no extension, so the category is what says it is
	// a texture; everywhere else the extension does.
	const bool is_texture = IsImageExt(name) ||
							(m_Source == 1 && (kCategories[m_Category].id == smTexture ||
											   kCategories[m_Category].id == smTextureRaw));
	if (is_texture)
	{
		xr_string img_err;
		if (UIImagePreview::Show(name, m_Source, &img_err))	return true;
		OPEN_FAILED("%s: %s", name, img_err.c_str());
	}

	OPEN_FAILED("no viewer for this asset kind yet: '%s'", name);
#undef OPEN_FAILED
}

// depth-first lookup by full path; null when the path is not in the tree
UIContentBrowser::SFolder* UIContentBrowser::FindFolder(LPCSTR path)
{
	if (!path || !path[0]) return &m_Root;

	xr_vector<SFolder*> stack; stack.push_back(&m_Root);
	while (!stack.empty())
	{
		SFolder* f = stack.back(); stack.pop_back();
		if (0 == _stricmp(f->path.c_str(), path)) return f;
		for (u32 i = 0; i < f->children.size(); ++i) stack.push_back(&f->children[i]);
	}
	return 0;
}

void UIContentBrowser::InvalidateView()
{
	m_ViewDirty = true;
}

// the search has something to look for: spaces alone do not count
static bool SearchActive(LPCSTR query)
{
	for (; query && *query; ++query)
		if (' ' != *query && '\t' != *query)
			return true;
	return false;
}

void UIContentBrowser::RebuildView()
{
	if (!m_ViewDirty) return;
	m_ViewDirty = false;
	m_ViewOrder.clear();
	m_ViewFolders = m_ViewFiles = 0;

	SFolder* cur = FindFolder(m_CurFolder.c_str());
	if (!cur) cur = &m_Root;

	// A search lists the folder shown and everything below it, matched on the path under that folder:
	// "ui cursor" finds textures\ui\cursor.dds from textures.
	const bool searching = SearchActive(m_Search);
	const size_t below = cur->path.empty() ? 0 : cur->path.size() + 1;
	auto passes = [&](LPCSTR path)
	{
		return !searching || XUI::Match(m_Search, xr_strlen(path) > below ? path + below : path);
	};

	if (!searching)
	{
		m_ViewOrder.reserve(cur->children.size() + cur->items.size());
		for (u32 i = 0; i < cur->children.size(); ++i)
			m_ViewOrder.push_back(SEntry(cur->children[i].path.c_str(), true));
	}
	else
	{
		xr_vector<SFolder*> stack;
		for (u32 i = cur->children.size(); i-- > 0;)
			stack.push_back(&cur->children[i]);
		while (!stack.empty())
		{
			SFolder* f = stack.back();
			stack.pop_back();
			if (passes(f->path.c_str()))
				m_ViewOrder.push_back(SEntry(f->path.c_str(), true));
			for (u32 i = f->children.size(); i-- > 0;)
				stack.push_back(&f->children[i]);
		}
	}
	m_ViewFolders = int(m_ViewOrder.size());

	xr_vector<int> ids;
	CollectItems(*cur, ids, searching);
	m_ViewOrder.reserve(m_ViewOrder.size() + ids.size());
	for (u32 i = 0; i < ids.size(); ++i)
	{
		LPCSTR full = m_Items[ids[i]].name.c_str();
		if (passes(full))
			m_ViewOrder.push_back(SEntry(full, false));
	}
	m_ViewFiles = int(m_ViewOrder.size()) - m_ViewFolders;
}

void UIContentBrowser::OpenEntry(const SEntry& entry)
{
	// a copy: the navigation below clears the selection this may point into
	const SEntry target = entry;
	if (!target.folder)
	{
		SelectEntry(target.path.c_str(), false, false, false);
		OpenAsset(target.path.c_str());
		return;
	}
	// a game level is a folder on disk and a scene to the author
	string_path level_scene;
	if (IsGameLevelEntry(target.path.c_str()) && !strchr(target.path.c_str() + 7, '\\') &&
		ResolveLevelFile(target.path.c_str(), m_Source, level_scene))
	{
		OpenAsset(target.path.c_str());
		return;
	}
	// into the folder a search found: the search has done its job
	m_Search[0] = 0;
	xr_string err;
	ApplyLocation(SLocation(m_Source, int(m_Category), target.path.c_str()), true, err);
}

void UIContentBrowser::DrawTiles()
{
	RebuildView();

	// Only visible rows have screen rectangles. Range selection and Ctrl+A use
	// the cached complete view instead, so virtualization cannot change meaning.
	m_DrawnOrder.clear();
	m_DrawnRects.clear();

	const ImGuiIO& io = ImGui::GetIO();
	// Ctrl+wheel sizes the tiles, and the list is the step under the smallest one (Explorer's zoom); the grid does not
	// scroll meanwhile (NoScrollWithMouse under Ctrl)
	if (io.KeyCtrl && io.MouseWheel != 0.f && ImGui::IsWindowHovered())
	{
		// a step from the size shown picks a size: a fitted host stops fitting
		const float size = ShownTile(m_Host);
		if (m_ListView[m_Host])
		{
			if (io.MouseWheel > 0.f)
			{
				m_ListView[m_Host]	= false;
				m_TileSize[m_Host]	= EditorUiContract::kTileMin;
			}
		}
		else if (io.MouseWheel < 0.f && size <= EditorUiContract::kTileMin)
			m_ListView[m_Host] = true;
		else
			m_TileSize[m_Host] = clampr(size + io.MouseWheel * 8.f, EditorUiContract::kTileMin, EditorUiContract::kTileMax);
		StoreView();
	}

	const int total = int(m_ViewOrder.size());
	if (!total)
	{
		if (SearchActive(m_Search))
			XUI::EmptyState(XUI::Icon::Search, "No matches", "Nothing in this folder or below matches the search.");
		else if (0 == m_Source && !EditorProject::Active())
			XUI::EmptyState(XUI::Icon::Folder, "No project open");
		else
			XUI::EmptyState(XUI::Icon::FolderOpen, "Empty folder",
				0 == m_Source ? "Use Add to create a folder or an asset here." : nullptr);
		DrawGridContextMenu();
		return;
	}

	ImDrawList* dl = ImGui::GetWindowDrawList();
	const ImGuiStyle& st = ImGui::GetStyle();
	const float grid_x = ImGui::GetCursorScreenPos().x;
	const float avail = _max(1.f, ImGui::GetContentRegionAvail().x);
	const bool list = m_ListView[m_Host];
	const float gap = XUI::Pt(kTileGap);
	const float pad = XUI::Pt(kTilePad);
	// Tiles: a square with a two-line caption under it; a tile never outgrows the grid, a narrow browser shows one
	// smaller column, not a clipped one, and a host with no size picked fits two rows (ShownTile). The list: rows of
	// one line in columns of 12 em at least, stretched to the width - six rows where the Wide layout's strip holds
	// under two rows of the smallest tiles.
	const float tile_w = _max(XUI::Pt(24.f), _min(XUI::Pt(ShownTile(m_Host)), avail));
	ImFont* caption_font = list ? ImGui::GetFont() : XUI::GetFont(XUI::Font::Small);
	if (!caption_font) caption_font = ImGui::GetFont();
	const float caption_px = list ? ImGui::GetFontSize() : XUI::Pt(XUI::FontSize(XUI::Font::Small));
	const float picture = XUI::Pt(kListPicture);
	int per_row;
	float cell_w, cell_h, row_h;
	if (list)
	{
		per_row	= _max(1, int((avail + gap) / (XUI::Em(kListColumnEm) + gap)));
		cell_w	= _max(1.f, floorf((avail - gap * float(per_row - 1)) / float(per_row)));
		cell_h	= XUI::Pt(kListRow);
		row_h	= cell_h;
	}
	else
	{
		per_row	= _max(1, int((avail + gap) / (tile_w + gap)));
		cell_w	= tile_w;
		cell_h	= tile_w + caption_px * 2.f + pad;
		row_h	= cell_h + gap;
	}
	const bool focused = ImGui::IsWindowFocused(ImGuiFocusedFlags_RootAndChildWindows);
	const bool project = 0 == m_Source && EditorProject::Active();
	const u32 category = kCategories[m_Category].id;

	int reveal_index = -1;
	if (m_ScrollToSelection)
		for (int i = total - 1; i >= 0; --i)
			if (IsSelected(m_ViewOrder[i].path.c_str(), m_ViewOrder[i].folder))
			{
				reveal_index = i;
				break;
			}
	if (reveal_index < 0 && !m_Rename.empty())
		for (int i = 0; i < total; ++i)
			if (m_ViewOrder[i].path == m_Rename)
			{
				reveal_index = i;
				break;
			}

	// A new folder starts at its beginning. ImGui keeps the scroll offset per
	// child window, so without this, entering a folder from a scrolled grid
	// lands in the middle of the new one - it looks like the first rows simply
	// do not exist. Unreal resets the same way.
	if (m_ShownFolder != m_CurFolder)
	{
		m_ShownFolder = m_CurFolder;
		if (reveal_index < 0) ImGui::SetScrollY(0.f);
	}

	// what a double click asks for runs once the grid is drawn: it may change what the grid lists
	int open_index = -1;
	bool rename_drawn = false;
	// where the rename box goes: over a tile's caption, or over a list row's name
	ImVec2 rename_at(0.f, 0.f);
	float rename_w = 0.f;

	ImGuiListClipper clipper;
	clipper.Begin((total + per_row - 1) / per_row, row_h);
	if (reveal_index >= 0)
		clipper.IncludeItemsByIndex(reveal_index / per_row, reveal_index / per_row + 1);
	while (clipper.Step())
		for (int row = clipper.DisplayStart; row < clipper.DisplayEnd; ++row)
		{
			const ImVec2 origin = ImGui::GetCursorScreenPos();
			const int first = row * per_row;
			const int last = _min(first + per_row, total);
			for (int index = first; index < last; ++index)
			{
				const SEntry& entry = m_ViewOrder[index];
				LPCSTR full = entry.path.c_str();
				LPCSTR leaf = strrchr(full, '\\');
				leaf = leaf ? leaf + 1 : full;
				// the editor's own assets go by their name alone: the icon already says the kind
				LPCSTR leaf_end = entry.folder ? nullptr : EditorAssetIcons::OwnExtension(leaf);
				const bool sel = IsSelected(full, entry.folder);
				const bool renaming = !m_Rename.empty() && m_Rename == full;
				// what the build does with it: a veiled tile does not ship
				xr_string ship_rule;
				const EditorMod::EShipState ship = project
					? EditorMod::ShipState(EditorProject::Root(), full, &ship_rule) : EditorMod::shipShipped;
				const bool veiled = ship == EditorMod::shipExcluded || ship == EditorMod::shipExcludedByRule;

				const ImVec2 a(origin.x + float(index - first) * (cell_w + gap), origin.y);
				const ImVec2 b(a.x + cell_w, a.y + cell_h);
				// the path is the id: a double click that walks into a folder must not land as a click
				// on the tile the new folder happens to put under the cursor
				ImGui::PushID(full);
				ImGui::SetCursorScreenPos(a);
				// the rename box drawn over this tile later takes the clicks inside it
				if (renaming)
					ImGui::SetNextItemAllowOverlap();
				const bool clicked = ImGui::InvisibleButton("##tile", ImVec2(cell_w, cell_h));
				const bool hovered = ImGui::IsItemHovered();
				m_DrawnOrder.push_back(entry);
				m_DrawnRects.push_back(ImVec4(a.x, a.y, b.x, b.y));

				// the picture's square: a tile's own, or the one a list row starts with
				ImVec2 sa, sb;
				if (list)
				{
					sa = ImVec2(PxRound(a.x + XUI::Pt(3.f)), PxRound(a.y + (cell_h - picture) * .5f));
					sb = ImVec2(sa.x + picture, sa.y + picture);
				}
				else
				{
					sa = ImVec2(a.x + pad, a.y + pad);
					sb = ImVec2(b.x - pad, a.y + tile_w - pad);
				}
				// the picture, or the glyph of what it is
				SThumb thumb;
				if (!entry.folder)
					if (const SThumb* cached = GetThumb(full, sb.x - sa.x))
						thumb = *cached;
				ImTextureID tex = thumb.tex;
				const EditorAssetIcons::EKind own = (entry.folder || tex) ? EditorAssetIcons::kNone : IconOf(full, m_Source);

				const float r = XUI::Pt(4.f);
				if (sel)
					dl->AddRectFilled(a, b, ThemeCol(focused ? Token::SelectionFocused : Token::Selection), r);
				else if (hovered)
					dl->AddRectFilled(a, b, ThemeCol(Token::Hover), r);
				if (!sel && !list)
					dl->AddRectFilled(sa, sb, ThemeCol(Token::Raised), XUI::Pt(3.f));
				if (tex)
					DrawPicture(dl, tex, thumb.aspect, thumb.alpha != 0, sa, sb);
				else if (own)
					EditorAssetIcons::Draw(dl, own, sa.x, sa.y, sb.x, sb.y);
				else if (list)
				{
					// a list row's glyph at the 16 pt of every other list's icons
					const float g = XUI::Pt(16.f);
					const ImVec2 ga(PxRound((sa.x + sb.x - g) * .5f), PxRound((sa.y + sb.y - g) * .5f));
					XUI::DrawIconRect(dl, TileIcon(full, entry.folder, m_Source, category), ga, ImVec2(ga.x + g, ga.y + g),
						ThemeCol(Token::TextSecondary));
				}
				else
					DrawTileGlyph(dl, TileIcon(full, entry.folder, m_Source, category), sa, sb, ThemeCol(Token::TextSecondary));
				// over the picture, under the outline: a left-out tile still reads as selected
				if (veiled)
					dl->AddRectFilled(sa, sb, ThemeCol(Token::Inset, .6f), XUI::Pt(3.f));
				if (sel && focused)
					dl->AddRect(a, b, ThemeCol(Token::AccentBright), r, 0, _max(1.f, XUI::Pt(1.5f)));
				const float text_x = list ? sb.x + XUI::Pt(6.f) : a.x + pad;
				if (renaming)
				{
					rename_drawn	= true;
					rename_at		= list ? ImVec2(text_x, PxRound(a.y + (cell_h - ImGui::GetFrameHeight()) * .5f))
										   : ImVec2(a.x, a.y + tile_w);
					rename_w		= list ? b.x - text_x : tile_w;
				}
				else
					DrawTileCaption(dl, caption_font, caption_px,
						ImVec2(text_x, list ? PxRound(a.y + (cell_h - caption_px) * .5f) : a.y + tile_w),
						list ? _max(1.f, b.x - XUI::Pt(4.f) - text_x) : tile_w - pad * 2.f, list ? 1 : 2,
						ThemeCol(veiled ? Token::TextMuted : Token::Text), leaf, leaf_end, !entry.folder && !leaf_end, !list);

				if (clicked)
					SelectEntry(full, entry.folder, io.KeyCtrl, io.KeyShift);
				if (hovered && ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left))
					open_index = index;
				DrawEntryContextMenu(full, entry.folder);
				if (hovered && !renaming)
				{
					xr_string tip = full;
					if (thumb.src_w)
					{
						char size[64];
						sprintf_s(size, "\n%u x %u%s", thumb.src_w, thumb.src_h,
							2 == thumb.alpha ? ", fully transparent" : 1 == thumb.alpha ? ", partly transparent" : "");
						tip += size;
					}
					if (entry.folder && IsGameLevelEntry(full) && !strchr(full + 7, '\\'))
						tip += "\nA game level: a double click opens the scene it was built from.";
					if (own)
					{
						tip += "\n";
						tip += EditorAssetIcons::Name(own);
					}
					if (veiled)
						tip += "\nNot shipped: excluded by '" + ship_rule + "' in mod.ltx [exclude].";
					else if (ship == EditorMod::shipNever)
						tip += "\nA source: never shipped.";
					XUI::Tip(tip.c_str());
				}
				if (!entry.folder && ImGui::BeginDragDropSource())
				{
					if (!IsSelected(full, false)) SelectEntry(full, false, false, false);
					m_Dragged = full;
					ImGui::SetDragDropPayload(CB_DND_PAYLOAD, full, xr_strlen(full) + 1);
					const float side = XUI::Pt(48.f);
					const ImVec2 at = ImGui::GetCursorScreenPos();
					ImGui::Dummy(ImVec2(side, side));
					ImDrawList* tip_dl = ImGui::GetWindowDrawList();
					if (tex)
						DrawPicture(tip_dl, tex, thumb.aspect, thumb.alpha != 0, at, ImVec2(at.x + side, at.y + side));
					else if (own)
						EditorAssetIcons::Draw(tip_dl, own, at.x, at.y, at.x + side, at.y + side);
					else
						DrawTileGlyph(tip_dl, TileIcon(full, false, m_Source, category), at, ImVec2(at.x + side, at.y + side),
							ThemeCol(Token::TextSecondary));
					ImGui::TextUnformatted(leaf, leaf_end);
					if (m_Selection.size() > 1)
					{
						char more[48];
						sprintf_s(more, "and %d more", int(m_Selection.size()) - 1);
						ImGui::TextUnformatted(more);
					}
					ImGui::EndDragDropSource();
				}
				ImGui::PopID();
			}
			ImGui::SetCursorScreenPos(origin);
			ImGui::Dummy(ImVec2(avail, row_h - st.ItemSpacing.y));
		}

	// The inline rename box goes over the grid, up to 24 em wide, so a long name is seen as it is typed; it
	// is drawn last to sit above the neighbouring tiles. Centred under a tile, from the name in a list row.
	if (rename_drawn)
	{
		const float want = ImGui::CalcTextSize(m_RenameBuf).x + st.FramePadding.x * 2.f + XUI::Pt(12.f);
		const float w = _min(avail, clampr(want, rename_w, _max(rename_w, XUI::Em(24.f))));
		const float x = clampr(list ? rename_at.x : rename_at.x + (rename_w - w) * .5f, grid_x,
			_max(grid_x, grid_x + avail - w));
		ImGui::SetCursorScreenPos(ImVec2(PxRound(x), rename_at.y));
		ImGui::SetNextItemWidth(w);
		if (m_RenameFocus)
		{
			ImGui::SetKeyboardFocusHere();
			m_RenameFocus = false;
		}
		// The NAME is selected, not the extension - what every file browser does, so
		// typing replaces the name and leaves the kind of the file alone. ImGui selects
		// everything when a box is focused from code; the callback has the last word.
		struct Select
		{
			static int Stem(ImGuiInputTextCallbackData* data)
			{
				int& want = *(int*)data->UserData;
				if (want >= 0)
				{
					data->SelectionStart	= 0;
					data->SelectionEnd		= data->CursorPos = _min(want, data->BufTextLen);
					want = -1;
				}
				return 0;
			}
		};
		const bool done = ImGui::InputText("##cb_rename", m_RenameBuf, sizeof(m_RenameBuf),
			ImGuiInputTextFlags_EnterReturnsTrue | ImGuiInputTextFlags_CallbackAlways,
			&Select::Stem, &m_RenameSelect);
		if (done || ImGui::IsItemDeactivated()) CommitRename();
	}

	if (open_index >= 0)
		OpenEntry(m_ViewOrder[open_index]);

	// Shift-click needed the complete cached grid order, so it is resolved here.
	ApplyPendingRange();

	// A reveal brings the grid to what it selected: the LAST selected tile in
	// grid order, so a multi-item reveal lands on the end of the run instead of
	// jumping back to its start. The rects were captured as the tiles drew, and
	// a tile that is already on screen is left alone.
	if (m_ScrollToSelection)
		for (int i = _min(int(m_DrawnOrder.size()), int(m_DrawnRects.size())) - 1; i >= 0; --i)
			if (IsSelected(m_DrawnOrder[i].path.c_str(), m_DrawnOrder[i].folder))
			{
				ScrollRectIntoView(m_DrawnRects[i].y, m_DrawnRects[i].w);
				break;
			}

	// empty-space right-click; the shortcuts run from DrawBody() instead, so Del
	// and Ctrl+C reach the folder tree as well as the grid
	DrawGridContextMenu();

	// clicking empty space clears the selection, as every file browser does -
	// but only a bare click, so it does not fight the rubber-band below
	if (!m_Marquee &&
		ImGui::IsWindowHovered(ImGuiHoveredFlags_ChildWindows) &&
		!ImGui::IsAnyItemHovered() && !ImGui::IsAnyItemActive() &&
		ImGui::IsMouseClicked(ImGuiMouseButton_Left) &&
		!io.KeyCtrl)
		ClearSelection();

	UpdateMarquee();
}

//------------------------------------------------------------------------------
// Tile right-click. ONE menu for folders and assets alike: Copy, Cut, Paste and
// Delete mean the same thing whatever a tile holds, and a mixed selection is a
// perfectly ordinary selection. Only the entries that are genuinely about one
// kind - opening an asset's viewer, placing it in the scene - look at `folder`.
//
// Read-only contract: the SDK library and the linked game install can be copied
// OUT of and nothing else. Cut and Delete write, so they are refused there.
//------------------------------------------------------------------------------
void UIContentBrowser::DrawEntryContextMenu(LPCSTR path, bool folder)
{
	// MCP ui_popup cb_context:<path> puts it up the way a right click does, at the entry
	if (XUI::Shell::TakePopup((xr_string("cb_context:") + path).c_str()))
		XUI::Shell::OpenPopupAt("cb_entry_ctx", ImGui::GetItemRectMax());
	if (!ImGui::BeginPopupContextItem("cb_entry_ctx")) return;

	// right-clicking outside the selection moves it here, as Unreal does;
	// right-clicking inside keeps the multi-selection intact
	if (!IsSelected(path, folder)) SelectEntry(path, folder, false, false);

	const int count = int(m_Selection.size());
	const bool writable = 0 == m_Source;
	LPCSTR read_only = "This source is read-only: copy out of it into the project.";

	// what the menu acts on
	{
		LPCSTR leaf = strrchr(path, '\\');
		char head[64];
		sprintf_s(head, "%d selected", count);
		xr_string shown;
		if (!XUI::EllipsizeText(count > 1 ? head : (leaf ? leaf + 1 : path), XUI::Em(20.f), XUI::Clip::End, shown))
			shown = count > 1 ? head : (leaf ? leaf + 1 : path);
		ImGui::PushStyleColor(ImGuiCol_Text, ThemeCol(Token::TextMuted));
		ImGui::TextUnformatted(shown.c_str());
		ImGui::PopStyleColor();
		ImGui::Separator();
	}

	if (XUI::MenuItem("Open", "Enter"))
		OpenEntry(SEntry(path, folder));
	if (!folder)
	{
		string_path ref;
		const bool placeable = IsGameSource() ? EditorGameContent::ResolvePlaceable(path, ref) : true;
		if (ImGui::MenuItem("Place in Scene", nullptr, false, placeable))
			ExecCommand(COMMAND_CB_PLACE_ASSET, xr_string(IsGameSource() ? ref : path), u32(0));
		if (!placeable)
			XUI::Tip("Only a library object reference is placed from the game's content.");
	}
	ImGui::Separator();

	// ---- the common commands ---------------------------------------------
	int n = 0;
	// a library category with no file of its own (particles, light anims)
	// has nothing to hand over; everything else is copyable
	const bool copyable = (m_Source != 1) || !!LibFilesFor(kCategories[m_Category].id, n);
	if (XUI::MenuItem("Copy", "Ctrl+C", false, copyable))
		ClipboardCopy(false);
	if (!copyable)
		XUI::Tip("This category lives inside one shared library file: there is no file of its own to copy.");
	if (XUI::MenuItem("Cut", "Ctrl+X", false, writable))
		ClipboardCopy(true);
	if (!writable)
		XUI::Tip(read_only);
	const bool can = CanPaste();
	if (XUI::MenuItem("Paste", "Ctrl+V", false, can))
		ClipboardPaste();
	if (!can)
		XUI::Tip(PasteBlockedReason());
	if (XUI::MenuItem("Rename", "F2", false, writable && 1 == count))
		BeginRename(path);
	if (!writable)
		XUI::Tip(read_only);
	else if (1 != count)
		XUI::Tip("One entry is renamed at a time.");
	// the item that asks is quiet, in the error colour; the loud button is the confirmation's
	const bool erase = XUI::MenuItem("Delete...", XUI::Icon::None, "Del", writable, true);
	if (erase)
		DeleteSelection();
	if (!writable)
		XUI::Tip(read_only);

	if (writable && EditorProject::Active())
	{
		ImGui::Separator();
		DrawBuildToggle();
	}

	ImGui::Separator();
	if (ImGui::MenuItem("Copy Path"))
		ImGui::SetClipboardText(path);
	if (writable && ImGui::MenuItem("Show in Explorer"))
	{
		xr_string where, err;
		if (!RevealInExplorer(path, false, where, err))
			XUI::Toast(XUI::Tone::Warning, err.c_str());
	}
	ImGui::EndPopup();
}

// The project folder is the author's working place as well as the module - design
// notes, a git repository, reference pictures - so what ships is a choice made here,
// per entry, and kept in mod.ltx [exclude] with the repository.
void UIContentBrowser::DrawBuildToggle()
{
	LPCSTR root = EditorProject::Root();
	int shipped = 0, own = 0, never = 0;
	xr_string by;
	for (u32 i = 0; i < m_Selection.size(); ++i)
	{
		xr_string rule;
		switch (EditorMod::ShipState(root, m_Selection[i].path.c_str(), &rule))
		{
		case EditorMod::shipShipped:			++shipped;		break;
		case EditorMod::shipExcluded:			++own;			break;
		case EditorMod::shipExcludedByRule:		by = rule;		break;
		default:								++never;		break;
		}
	}
	if (never && !shipped && !own && by.empty())
	{
		ImGui::MenuItem("Exclude from Build", nullptr, false, false);
		XUI::Tip("The editor's own files and the sources under rawdata are never shipped.");
		return;
	}
	xr_string err;
	if (shipped)
	{
		if (ImGui::MenuItem("Exclude from Build"))
			for (u32 i = 0; i < m_Selection.size() && err.empty(); ++i)
				if (EditorMod::shipShipped == EditorMod::ShipState(root, m_Selection[i].path.c_str()))
					EditorMod::SetExcluded(root, m_Selection[i].path.c_str(), true, err);
		XUI::Tip("Left out of the module and its releases, kept in the project. Stored in mod.ltx [exclude]; "
				 "Mod Settings lists them.");
	}
	else if (own && by.empty())
	{
		if (ImGui::MenuItem("Include in Build"))
			for (u32 i = 0; i < m_Selection.size() && err.empty(); ++i)
				EditorMod::SetExcluded(root, m_Selection[i].path.c_str(), false, err);
	}
	else
	{
		ImGui::MenuItem("Include in Build", nullptr, false, false);
		const xr_string why = "Excluded by '" + by + "' in mod.ltx [exclude]: include that one instead.";
		XUI::Tip(why.c_str());
	}
	if (!err.empty())
		XUI::Notice(XUI::Tone::Error, "Build Contents", err.c_str());
}

bool UIContentBrowser::ResolveDropRef(LPCSTR name, string_path& ref)
{
	ref[0] = 0;
	if (!name || !name[0])	return false;
	const int source = Form ? Form->m_Source : 1;

	// the linked game install: only an .object the library can load qualifies
	if (2 == source)
		return EditorGameContent::ResolvePlaceable(name, ref);

	// the project: its rawdata\objects mirrors the library layout, so the ref
	// is the path below that root with the extension clamped off
	if (0 == source)
	{
		LPCSTR ext = strrchr(name, '.');
		if (!ext || 0 != _stricmp(ext, ".object"))	return false;

		LPCSTR rel = name;
		if (0 == _strnicmp(rel, "rawdata\\objects\\", 16))	rel += 16;
		else return false;

		xr_strcpy(ref, sizeof(string_path), rel);
		if (char* dot = strrchr(ref, '.'))	*dot = 0;
		return true;
	}

	// the SDK library: an Objects entry IS a reference; nothing else places
	if (Form && kCategories[Form->m_Category].id == smObject)
	{
		xr_strcpy(ref, sizeof(string_path), name);
		return true;
	}
	return false;
}

// The other placeable thing: a compiled engine model. A .ogf out of the
// project or the linked game (or a Visuals entry of the SDK library, which
// is the same thing by name) becomes a raw-visual scene object - the drop
// target passes the path through to COMMAND_CB_PLACE_ASSET with the visual
// bit set, and CSceneObject::SetVisual does the rest.
bool UIContentBrowser::ResolveDropVisual(LPCSTR name, string_path& path)
{
	path[0] = 0;
	if (!name || !name[0])	return false;
	const int source = Form ? Form->m_Source : 1;

	LPCSTR ext = strrchr(name, '.');
	if ((0 == source || 2 == source) && ext && 0 == _stricmp(ext, ".ogf"))
	{
		xr_strcpy(path, sizeof(string_path), name);
		return true;
	}
	if (1 == source && Form && kCategories[Form->m_Category].id == smVisual)
	{
		xr_strcpy(path, sizeof(string_path), name);
		return true;
	}
	return false;
}

//------------------------------------------------------------------------------
// MCP entry points
//------------------------------------------------------------------------------
bool UIContentBrowser::RevealAsset(LPCSTR name, int source, bool open_viewer, xr_string& err)
{
	if (!name || !name[0])	{ err = "empty asset name"; return false; }
	Show();									// the panel has to exist to show anything
	if (!Form)				{ err = "content browser unavailable"; return false; }
	const SLocation origin = Form->CurrentLocation();

	if (source >= 0 && source <= 2 && source != Form->m_Source)
		Form->SwitchSource(source);

	// the listing is rebuilt lazily; the lookup below needs it now
	Form->EnsureListing();

	int found = -1;
	// The listing follows the disk through a watcher polled from Draw(), a debounce
	// behind it - and a background editor draws nothing at all. A caller that has just
	// written the file is right: on a miss the project is listed again, once.
	for (int pass = 0; pass < 2 && found < 0; ++pass)
	{
		for (u32 i = 0; i < Form->m_Items.size(); ++i)
			if (0 == _stricmp(Form->m_Items[i].name.c_str(), name)) { found = int(i); break; }

		// A folder is a legitimate thing to reveal - "show me levels\" is what a
		// caller browsing the tree actually wants, and there is no other way to
		// get the grid there without a mouse. Selected like a folder tile, so the
		// next frame scrolls the tree to it as well.
		if (found < 0)
		{
			string_path folder;
			strncpy_s(folder, sizeof(folder), name, _TRUNCATE);
			for (char* p = folder; *p; ++p) if (*p == '/') *p = '\\';
			if (const size_t n = xr_strlen(folder); n && folder[n - 1] == '\\') folder[n - 1] = 0;
			if (Form->FindFolder(folder))
			{
				xr_string nav_err;
				if (!Form->ApplyLocation(SLocation(Form->m_Source, int(Form->m_Category), folder), false, nav_err))
				{
					err = nav_err;
					return false;
				}
				Form->RememberNavigation(origin);
				Form->m_Search[0] = 0;
				Form->InvalidateView();
				Form->ClearSelection();
				Form->AddToSelection(SEntry(Form->m_CurFolder.c_str(), true));
				Form->m_Anchor		= Form->m_Selection.back();
				Form->m_HasAnchor	= true;
				Form->m_ScrollToSelection = true;
				// a game level is a folder on disk but a level to open
				if (open_viewer && Form->IsGameLevelEntry(Form->m_CurFolder.c_str()))
					return Form->OpenAsset(Form->m_CurFolder.c_str(), &err);
				return true;
			}
		}
		if (found >= 0 || pass || 0 != Form->m_Source)	break;
		Form->Refresh();
	}

	// The SDK library is split into categories, and a caller naming an asset has
	// no reason to know which one holds it - a level lives under Levels, a mesh
	// under Visuals. Sweep them rather than answering "not found" for something
	// that is plainly there.
	if (found < 0 && 1 == Form->m_Source)
	{
		const SLocation sweep_location = Form->CurrentLocation();
		const xr_vector<SEntry> sweep_selection = Form->m_Selection;
		const SEntry sweep_anchor = Form->m_Anchor;
		const SEntry sweep_pending_range = Form->m_PendingRange;
		const bool sweep_has_anchor = Form->m_HasAnchor;
		const bool sweep_has_pending_range = Form->m_HasPendingRange;
		const bool sweep_scroll_to_selection = Form->m_ScrollToSelection;
		for (int c = 0; c < kCategoryCount && found < 0; ++c)
		{
			if (c == sweep_location.category)	continue;
			Form->m_Category	= u32(c);
			Form->m_CurFolder	= "";
			Form->Refresh		();
			for (u32 i = 0; i < Form->m_Items.size(); ++i)
				if (0 == _stricmp(Form->m_Items[i].name.c_str(), name)) { found = int(i); break; }
		}
		if (found < 0)
		{
			Form->m_Source = sweep_location.source;
			Form->m_Category = u32(sweep_location.category);
			Form->m_CurFolder = sweep_location.folder;
			Form->Refresh();
			Form->m_Selection = sweep_selection;
			Form->RebuildSelectionLookup();
			Form->m_Anchor = sweep_anchor;
			Form->m_PendingRange = sweep_pending_range;
			Form->m_HasAnchor = sweep_has_anchor;
			Form->m_HasPendingRange = sweep_has_pending_range;
			Form->m_ScrollToSelection = sweep_scroll_to_selection;
		}
	}

	if (found < 0)
	{
		err = "asset not found in this source";
		return false;
	}

	// navigate to the folder that holds it, clear any search that would hide it
	const xr_string found_name = Form->m_Items[found].name.c_str();
	LPCSTR	cut = strrchr(found_name.c_str(), '\\');
	const xr_string target_folder = cut ? found_name.substr(0, size_t(cut - found_name.c_str())) : xr_string("");
	xr_string nav_err;
	if (!Form->ApplyLocation(SLocation(Form->m_Source, int(Form->m_Category), target_folder.c_str()),
		false, nav_err))
	{
		err = nav_err;
		return false;
	}
	Form->RememberNavigation(origin);
	Form->m_Search[0] = 0;
	Form->InvalidateView();

	Form->ClearSelection();
	Form->AddToSelection(SEntry(found_name.c_str(), false));
	Form->m_Anchor		= Form->m_Selection.back();
	Form->m_HasAnchor	= true;
	// selecting it is only half a reveal - the next frame brings it into view
	Form->m_ScrollToSelection = true;

	// the reveal itself succeeded; a viewer that will not open is worth saying
	// out loud rather than reporting a bare ok
	if (open_viewer) return Form->OpenAsset(found_name.c_str(), &err);
	return true;
}

void UIContentBrowser::GetSelection(int& source, xr_string& folder, xr_vector<xr_string>& sel)
{
	source = Form ? Form->m_Source : -1;
	folder = Form ? Form->m_CurFolder : xr_string("");
	sel.clear();
	if (!Form) return;
	// folders come across with a trailing '\\', so a caller can still tell them
	// apart without the browser's own type leaking into the MCP surface
	for (u32 i = 0; i < Form->m_Selection.size(); ++i)
	{
		xr_string s(Form->m_Selection[i].path);
		if (Form->m_Selection[i].folder) s += "\\";
		sel.push_back(s);
	}
}

LPCSTR UIContentBrowser::IconName(LPCSTR entry)
{
	return EditorAssetIcons::Name(IconOf(entry, Form ? Form->m_Source : 0));
}

// The side a host's tiles are drawn at (pt). With no size picked they fit the host: the largest side up to its default
// at which the grid shows two whole rows, never under the smallest - the Wide layout's strip holds under two rows of
// even those, and at 48 pt it shows two rows of pictures with their first caption line, 4x what one row of 96 did.
float UIContentBrowser::ShownTile(int host) const
{
	if (m_TileSize[host] > 0.f)
		return m_TileSize[host];
	const float most = kHostPanel == host ? EditorUiContract::kTileDefaultPanel : EditorUiContract::kTileDefault;
	if (m_GridHeight[host] <= 0.f)
		return most;
	// two rows: 2 x (square + two caption lines + pad) and the gap between them; in the 8 pt steps of Ctrl+wheel, so
	// resizing the host remakes the pictures at a step, not at every pixel
	const float fit = (m_GridHeight[host] - kTileGap) * .5f - XUI::FontSize(XUI::Font::Small) * 2.f - kTilePad;
	return clampr(floorf(fit / 8.f) * 8.f, EditorUiContract::kTileMin, most);
}

float UIContentBrowser::TileSize()
{
	if (Form) Form->LoadView();
	return Form ? Form->ShownTile(TargetHost()) : 0.f;
}

bool UIContentBrowser::SetTileSize(float size, xr_string& err)
{
	if (!Form)						{ err = "content browser unavailable"; return false; }
	if (size < EditorUiContract::kTileMin || size > EditorUiContract::kTileMax)	{ err = EditorUiContract::kTileRangeError; return false; }
	Form->LoadView();
	Form->m_TileSize[TargetHost()] = size;
	Form->m_ListView[TargetHost()] = false;
	Form->StoreView();
	return true;
}

bool UIContentBrowser::TileFit()
{
	if (Form) Form->LoadView();
	return Form && Form->m_TileSize[TargetHost()] <= 0.f;
}

void UIContentBrowser::SetTileFit(bool on)
{
	if (!Form)
		return;
	Form->LoadView();
	const int host = TargetHost();
	// a tile size either way, and a size asks for tiles
	Form->m_TileSize[host] = on ? 0.f : Form->ShownTile(host);
	Form->m_ListView[host] = false;
	Form->StoreView();
}

bool UIContentBrowser::CanAdd()
{
	return Form && !Form->AddBlocked();
}

LPCSTR UIContentBrowser::ViewMode()
{
	if (Form) Form->LoadView();
	return Form && Form->m_ListView[TargetHost()] ? "list" : "tiles";
}

bool UIContentBrowser::SetViewMode(LPCSTR mode, xr_string& err)
{
	if (!Form)						{ err = "content browser unavailable"; return false; }
	const bool list = mode && 0 == _stricmp(mode, "list");
	if (!list && (!mode || 0 != _stricmp(mode, "tiles")))	{ err = "view must be tiles or list"; return false; }
	Form->LoadView();
	Form->m_ListView[TargetHost()] = list;
	Form->StoreView();
	return true;
}

LPCSTR UIContentBrowser::Glyph(LPCSTR entry)
{
	if (!entry || !entry[0])	return "";
	xr_string path = entry;
	const bool folder = '\\' == path.back() || '/' == path.back();
	if (folder)	path.pop_back();
	const int source = Form ? Form->m_Source : 0;
	const u32 category = Form ? kCategories[Form->m_Category].id : 0;
	return XUI::IconName(TileIcon(path.c_str(), folder, source, category));
}

void UIContentBrowser::SetSearch(LPCSTR text)
{
	Show();
	if (!Form)	return;
	xr_strcpy(Form->m_Search, sizeof(Form->m_Search), text ? text : "");
	Form->InvalidateView();
}

LPCSTR UIContentBrowser::Search()
{
	return Form ? Form->m_Search : "";
}

void UIContentBrowser::SetSourcesPane(bool on)
{
	Show();
	if (!Form)	return;
	Form->LoadView();
	Form->m_SourcesPane = on;
	Form->StoreView();
}

void UIContentBrowser::GetSourcesPane(bool& on, bool& shown)
{
	on = Form ? Form->m_SourcesPane : true;
	shown = Form && Form->m_SourcesShown;
}

bool UIContentBrowser::RevealInExplorer(LPCSTR entry, bool dry_run, xr_string& path, xr_string& err)
{
	path.clear();
	if (!EditorProject::Active())	{ err = "no project is open"; return false; }
	if (!entry || !entry[0])		{ err = "missing 'name'"; return false; }
	string_path rel;
	xr_strcpy(rel, sizeof(rel), entry);
	for (char* p = rel; *p; ++p) if ('/' == *p) *p = '\\';
	if (const size_t n = xr_strlen(rel); n && '\\' == rel[n - 1]) rel[n - 1] = 0;
	xr_string normalized;
	if (!NormalizeFolderPath(rel, normalized, err))	return false;
	if (normalized.empty())		{ err = "name the entry to select, not the project root"; return false; }
	if (!ContentAbsolutePathFits(EditorProject::Root(), normalized.size())) { err = "the path is too long"; return false; }
	path = EditorProject::Root();
	path += "\\";
	path += normalized;
	if (INVALID_FILE_ATTRIBUTES == ::GetFileAttributesA(path.c_str()))
	{
		err = "no such file or folder in the project";
		return false;
	}
	if (dry_run)	return true;

	// Explorer opens the folder that holds it, with it selected
	wchar_t wide[MAX_PATH * 2];
	if (!::MultiByteToWideChar(CP_ACP, 0, path.c_str(), -1, wide, int(_countof(wide))))
	{
		err = "the path cannot be handed to Explorer";
		return false;
	}
	std::wstring args = L"/select,\"";
	args += wide;
	args += L"\"";
	if (INT_PTR(::ShellExecuteW(nullptr, L"open", L"explorer.exe", args.c_str(), nullptr, SW_SHOWNORMAL)) <= 32)
	{
		err = "Explorer did not start";
		return false;
	}
	return true;
}

bool UIContentBrowser::GetRename(xr_string& entry, xr_string& text, int& select)
{
	if (!Form || Form->m_Rename.empty())	return false;
	entry	= Form->m_Rename;
	text	= Form->m_RenameBuf;
	select	= Form->m_RenameStem;
	return true;
}

bool UIContentBrowser::ModifySelection(LPCSTR action, LPCSTR name, xr_string& err)
{
	err = "";
	if (!action || !action[0])	{ err = "action must be select, toggle, range or clear"; return false; }
	Show();
	if (!Form)					{ err = "content browser unavailable"; return false; }

	if (0 == _stricmp(action, "clear"))
	{
		Form->ClearSelection();
		Form->m_ScrollToSelection = false;
		// an inline rename nobody finished goes with it, unrenamed
		Form->m_Rename.clear();
		Form->m_RenameFocus = false;
		return true;
	}

	const bool select = 0 == _stricmp(action, "select");
	const bool toggle = 0 == _stricmp(action, "toggle");
	const bool range = 0 == _stricmp(action, "range");
	// F2 for a caller without a keyboard: the box opens on the tile, holding what a hand
	// would find in it. The rename itself stays content_rename.
	const bool rename = 0 == _stricmp(action, "rename");
	if (!select && !toggle && !range && !rename)
	{
		err = "action must be select, toggle, range, rename or clear";
		return false;
	}
	if (!name || !name[0])	{ err = "this action requires 'name'"; return false; }

	char norm[512];
	strncpy_s(norm, sizeof(norm), name, _TRUNCATE);
	for (char* p = norm; *p; ++p) if ('/' == *p) *p = '\\';
	char* write = norm;
	for (const char* read = norm; *read; ++read)
		if (!(*read == '\\' && write > norm && write[-1] == '\\')) *write++ = *read;
	*write = 0;
	if (const size_t len = xr_strlen(norm); len && '\\' == norm[len - 1]) norm[len - 1] = 0;
	if (!norm[0])				{ err = "name resolves to an empty path"; return false; }

	Form->EnsureListing();
	Form->RebuildView();
	const SEntry* entry = 0;
	// one more look at the disk on a miss - see RevealAsset
	for (int pass = 0; pass < 2 && !entry; ++pass)
	{
		for (u32 i = 0; i < Form->m_ViewOrder.size(); ++i)
			if (0 == _stricmp(Form->m_ViewOrder[i].path.c_str(), norm))
			{
				if (entry)			{ err = "entry is ambiguous in the current view"; return false; }
				entry = &Form->m_ViewOrder[i];
			}
		if (entry || pass || 0 != Form->m_Source)	break;
		Form->Refresh();
		Form->RebuildView();
	}
	if (!entry)				{ err = "entry is not in the complete current view"; return false; }

	if (rename)
	{
		if (Form->m_Source != 0)	{ err = "only the project's own content is renamed"; return false; }
		const xr_string path = entry->path;
		Form->SelectEntry(path.c_str(), entry->folder, false, false);
		Form->BeginRename(path.c_str());
		Form->m_ScrollToSelection = true;
		return true;
	}

	if (range)
	{
		if (!Form->m_HasAnchor)	{ err = "range requires a selected anchor"; return false; }
		bool anchor_in_view = false;
		for (u32 i = 0; i < Form->m_ViewOrder.size(); ++i)
			if (Form->m_ViewOrder[i] == Form->m_Anchor)
			{
				anchor_in_view = true;
				break;
			}
		if (!anchor_in_view)		{ err = "selection anchor is not in the complete current view"; return false; }
		Form->SelectEntry(entry->path.c_str(), entry->folder, false, true);
		Form->ApplyPendingRange();
		Form->m_ScrollToSelection = true;
		return true;
	}

	Form->SelectEntry(entry->path.c_str(), entry->folder, toggle, false);
	Form->m_ScrollToSelection = select || Form->IsSelected(entry->path.c_str(), entry->folder);
	return true;
}

bool UIContentBrowser::McpCopyToProject(LPCSTR names, LPCSTR folder, LPCSTR dst,
										int source, int category,
										bool overwrite, int& files, xr_string& err)
{
	files	= 0;
	err		= "";

	if (!Form) Form = xr_new<UIContentBrowser>();

	const int target_source = source >= 0 ? source : Form->m_Source;
	const int target_category = category >= 0 ? category : int(Form->m_Category);
	if (target_category >= kCategoryCount)
	{
		err = "unknown category";
		return false;
	}
	if ((target_source != Form->m_Source || u32(target_category) != Form->m_Category) &&
		!Form->ApplyLocation(SLocation(target_source, target_category, ""), true, err))
		return false;
	if (Form->m_Source == 0)	{ err = "the project's own Content is a destination, not a source"; return false; }

	// the listing has to be current: the tree is what turns a folder into the
	// set of items under it, and a source switch leaves it stale
	if (Form->m_NeedRefresh) Form->Refresh();

	xr_vector<xr_string> list;
	if (folder && folder[0])
	{
		string_path norm;
		xr_strcpy	(norm, sizeof(norm), folder);
		for (char* p = norm; *p; ++p) if ('/' == *p) *p = '\\';

		SFolder* f = Form->FindFolder(norm);
		if (!f)	{ err = "no such folder in this source"; return false; }

		xr_vector<int> ids;
		Form->CollectItems(*f, ids, true);
		list.reserve(ids.size());
		for (u32 i = 0; i < ids.size(); ++i)
			list.push_back(Form->m_Items[ids[i]].name.c_str());
	}
	else if (names && names[0])
	{
		// ';'-separated, same convention the file-op commands use
		string_path buf;
		xr_strcpy	(buf, sizeof(buf), names);
		for (char* cursor = buf; cursor && *cursor;)
		{
			char* sep = strchr(cursor, ';');
			if (sep) *sep = 0;
			while (' ' == *cursor) ++cursor;
			for (char* e = cursor + xr_strlen(cursor); e > cursor && ' ' == e[-1];) *--e = 0;
			for (char* p = cursor; *p; ++p) if ('/' == *p) *p = '\\';
			if (*cursor) list.push_back(cursor);
			cursor = sep ? sep + 1 : 0;
		}
	}

	if (list.empty())	{ err = "nothing to copy - pass 'names' or 'folder'"; return false; }

	// No destination: mirror into the layout fs.ltx implies - the useful default
	// for a script that just wants an asset where the engine will find it.
	if (!dst || !dst[0])
	{
		files = Form->CopyRefsToProject(kCategories[Form->m_Category].id, list, overwrite, err);
		Form->m_NeedRefresh = true;
		return err.empty();
	}

	// With one, go through the browser's own clipboard and paste, so a scripted
	// copy and a menu copy cannot drift apart.
	Form->ClearSelection();
	if (folder && folder[0])
	{
		// the FOLDER goes on the clipboard, not the items under it - that is what
		// makes the paste rebuild the tree instead of flattening it
		string_path norm;
		xr_strcpy	(norm, sizeof(norm), folder);
		for (char* p = norm; *p; ++p) if ('/' == *p) *p = '\\';
		Form->AddToSelection(SEntry(norm, true));
	}
	else
		for (u32 i = 0; i < list.size(); ++i)
			Form->AddToSelection(SEntry(list[i].c_str(), false));
	Form->ClipboardCopy(false);
	Form->ClearSelection();

	string_path dst_root;
	xr_string	mk_err;
	if (!EditorFileOps::MakeDir(dst, dst_root, mk_err))	{ err = mk_err; return false; }

	files = Form->PasteClipboardInto(dst_root, overwrite, err);
	return err.empty();
}

//------------------------------------------------------------------------------
// selection
//------------------------------------------------------------------------------
bool UIContentBrowser::IsSelected(LPCSTR path, bool folder) const
{
	const SEntry e(path, folder);
	return m_SelectionLookup.find(e) != m_SelectionLookup.end();
}

void UIContentBrowser::AddToSelection(const SEntry& entry)
{
	if (m_SelectionLookup.emplace(entry, u8(1)).second)
		m_Selection.push_back(entry);
}

void UIContentBrowser::RebuildSelectionLookup()
{
	m_SelectionLookup.clear();
	m_SelectionLookup.reserve(m_Selection.size());
	for (u32 i = 0; i < m_Selection.size(); ++i)
		m_SelectionLookup.emplace(m_Selection[i], u8(1));
}

void UIContentBrowser::SelectAllViewEntries()
{
	RebuildView();
	m_Selection.clear();
	m_SelectionLookup.clear();
	m_Selection.reserve(m_ViewOrder.size());
	m_SelectionLookup.reserve(m_ViewOrder.size());
	for (u32 i = 0; i < m_ViewOrder.size(); ++i)
		AddToSelection(m_ViewOrder[i]);
	m_HasAnchor = !m_Selection.empty();
	if (m_HasAnchor) m_Anchor = m_Selection.back();
}

void UIContentBrowser::ClearSelection()
{
	m_Selection.clear();
	m_SelectionLookup.clear();
	m_HasAnchor			= false;
	m_HasPendingRange	= false;
}

void UIContentBrowser::SelectEntry(LPCSTR path, bool folder, bool additive, bool range)
{
	const SEntry e(path, folder);

	if (range && m_HasAnchor)
	{
		// Resolved after the click against the cached complete view order.
		m_PendingRange		= e;
		m_HasPendingRange	= true;
		return;
	}

	if (additive)
	{
		if (m_SelectionLookup.erase(e))
		{
			for (u32 i = 0; i < m_Selection.size(); ++i)
				if (m_Selection[i] == e)
				{
					m_Selection.erase(m_Selection.begin() + i);
					break;
				}
			// the anchor must stay on something that is still selected
			if (m_Anchor == e)
			{
				m_HasAnchor = !m_Selection.empty();
				if (m_HasAnchor) m_Anchor = m_Selection.back();
			}
			return;
		}
		AddToSelection(e);
		m_Anchor	= e;
		m_HasAnchor	= true;
		return;
	}

	ClearSelection();
	AddToSelection(e);
	m_Anchor	= e;
	m_HasAnchor	= true;
}

//------------------------------------------------------------------------------
// Rubber-band selection. Starts on empty grid space (a press over a tile is that
// tile's click, or the start of a drag&drop), extends while held, and selects
// everything the band touches. Ctrl keeps what was already selected, which is
// how Unreal and every file manager behave.
//------------------------------------------------------------------------------
void UIContentBrowser::UpdateMarquee()
{
	const ImGuiIO&	io		= ImGui::GetIO();
	const bool		inside	= ImGui::IsWindowHovered(ImGuiHoveredFlags_ChildWindows);

	if (!m_Marquee)
	{
		// only empty space starts a band, and never while a drag&drop is live
		if (inside && !ImGui::IsAnyItemHovered() && !ImGui::IsAnyItemActive() &&
			ImGui::IsMouseClicked(ImGuiMouseButton_Left))
		{
			m_Marquee		= true;
			m_MarqueeStart	= io.MousePos;
			m_MarqueeBase	= io.KeyCtrl ? m_Selection : xr_vector<SEntry>();
		}
		return;
	}

	const ImVec2 a(_min(m_MarqueeStart.x, io.MousePos.x), _min(m_MarqueeStart.y, io.MousePos.y));
	const ImVec2 b(_max(m_MarqueeStart.x, io.MousePos.x), _max(m_MarqueeStart.y, io.MousePos.y));

	// a click without movement is not a band - let the empty-space click clear
	const bool dragged = (b.x - a.x) > 3.f || (b.y - a.y) > 3.f;

	if (dragged)
	{
		m_Selection = m_MarqueeBase;
		RebuildSelectionLookup();
		for (u32 i = 0; i < m_DrawnRects.size() && i < m_DrawnOrder.size(); ++i)
		{
			const ImVec4& r = m_DrawnRects[i];
			// plain AABB overlap: touched counts as selected. Folder tiles are in
			// this list too - the band does not care what a tile holds.
			if (r.z < a.x || r.x > b.x || r.w < a.y || r.y > b.y)	continue;
			if (!IsSelected(m_DrawnOrder[i].path.c_str(), m_DrawnOrder[i].folder))
				AddToSelection(m_DrawnOrder[i]);
		}
		m_HasAnchor = !m_Selection.empty();
		if (m_HasAnchor) m_Anchor = m_Selection.back();

		// the look of a selected tile: the focused selection's fill, the accent's outline
		ImDrawList* dl = ImGui::GetWindowDrawList();
		dl->AddRectFilled(a, b, ThemeCol(Token::SelectionFocused, .35f));
		dl->AddRect      (a, b, ThemeCol(Token::AccentBright), 0.f, 0, _max(1.f, XUI::Pt(1.f)));
	}

	if (ImGui::IsMouseReleased(ImGuiMouseButton_Left))
	{
		m_Marquee = false;
		m_MarqueeBase.clear();
	}
}

void UIContentBrowser::ApplyPendingRange()
{
	if (!m_HasPendingRange)	return;

	int from = -1, to = -1;
	for (u32 i = 0; i < m_ViewOrder.size(); ++i)
	{
		if (m_ViewOrder[i] == m_Anchor)		from = int(i);
		if (m_ViewOrder[i] == m_PendingRange)	to	 = int(i);
	}
	m_HasPendingRange = false;
	if (from < 0 || to < 0)	return;
	if (from > to)			std::swap(from, to);

	m_Selection.clear();
	m_SelectionLookup.clear();
	for (int i = from; i <= to; ++i)
		AddToSelection(m_ViewOrder[i]);
	// the anchor stays put so dragging the range back and forth works
}

//------------------------------------------------------------------------------
// clipboard
//
// Ordinary file-manager semantics: copy marks what is selected and where it
// came from, paste materialises it in the folder you are looking at. Only the
// project's own Content can receive a paste - the SDK library and the game
// install are sources, never destinations.
//------------------------------------------------------------------------------
void UIContentBrowser::ClipExpandFolder(LPCSTR path)
{
	SFolder* f = FindFolder(path);
	if (!f)	return;

	LPCSTR leaf = strrchr(path, '\\');
	leaf = leaf ? leaf + 1 : path;
	// everything under the folder keeps its place BELOW the folder's own name,
	// so pasting it reproduces the tree rather than dumping it flat
	const size_t cut = xr_strlen(path);

	xr_vector<int> ids;
	CollectItems(*f, ids, true);
	for (u32 i = 0; i < ids.size(); ++i)
	{
		LPCSTR nm = m_Items[ids[i]].name.c_str();
		SClip c;
		c.src		= nm;
		c.folder	= false;
		c.rel		= leaf;
		// nm starts with path + '\\'
		if (xr_strlen(nm) > cut) { c.rel += "\\"; c.rel += (nm + cut + 1); }
		m_Clipboard.push_back(c);
	}
}

void UIContentBrowser::ClipboardCopy(bool cut)
{
	if (m_Selection.empty()) return;
	if (cut && m_Source != 0)
	{
		XUI::Toast(XUI::Tone::Info, "This source is read-only: Cut moves things, use Copy.");
		return;
	}

	m_Clipboard.clear();
	m_ClipSource	= m_Source;
	m_ClipCategory	= kCategories[m_Category].id;
	m_ClipCut		= cut;

	for (u32 i = 0; i < m_Selection.size(); ++i)
	{
		const SEntry& e = m_Selection[i];
		LPCSTR leaf = strrchr(e.path.c_str(), '\\');
		leaf = leaf ? leaf + 1 : e.path.c_str();

		// A project folder goes across as a folder: the file copier walks it
		// itself. In a read-only source there is no folder on disk to walk, so
		// it is expanded into the items the listing knows about.
		if (e.folder && m_Source != 0)	{ ClipExpandFolder(e.path.c_str()); continue; }

		SClip c;
		c.src		= e.path;
		c.rel		= leaf;
		c.folder	= e.folder;
		m_Clipboard.push_back(c);
	}

	ELog.Msg(mtInformation, "%s %d entr%s.", cut ? "Cut" : "Copied",
			 int(m_Selection.size()), 1 == m_Selection.size() ? "y" : "ies");
}

// Raises the confirmation, which names what goes. Nothing is touched until it
// is answered - a folder full of work used to go the moment Del was pressed,
// with no question asked and nothing to undo.
void UIContentBrowser::DeleteSelection()
{
	if (m_Selection.empty())	return;
	if (m_Source != 0 || !EditorProject::Active())
	{
		XUI::Toast(XUI::Tone::Info, "This source is read-only: nothing here is deleted.");
		return;
	}

	// name what is going, not just how much: "3 items" tells the user nothing
	// about whether the one they meant is among them
	int folders = 0, files = 0, inside = 0;
	XUI::Strings items;
	for (const SEntry& e : m_Selection)
	{
		items.push_back(e.folder ? e.path + "\\" : e.path);
		if (!e.folder)
		{
			++files;
			continue;
		}
		++folders;
		if (SFolder* f = FindFolder(e.path.c_str()))
		{
			xr_vector<int> ids;
			CollectItems(*f, ids, true);
			inside += int(ids.size());
		}
	}
	auto counted = [](int n, LPCSTR one, LPCSTR many) { return NqUtil::Format("%d %s", n, 1 == n ? one : many); };
	xr_string headline;
	if (1 == m_Selection.size())
	{
		LPCSTR leaf = strrchr(m_Selection[0].path.c_str(), '\\');
		leaf = leaf ? leaf + 1 : m_Selection[0].path.c_str();
		headline = folders ? "Delete the folder " + xr_string(leaf) + " and everything in it?" : "Delete " + xr_string(leaf) + "?";
	}
	else if (folders && files)
		headline = "Delete " + counted(folders, "folder", "folders") + " and " + counted(files, "file", "files") + "?";
	else
		headline = "Delete " + (folders ? counted(folders, "folder", "folders") : counted(files, "file", "files")) + "?";
	xr_string consequence;
	if (inside)
		consequence = (1 == folders ? "It holds " : "The folders hold ") + counted(inside, "file", "files") +
			". Deleted for good - this cannot be undone.";

	XUI::ConfirmSpec spec;
	spec.id				= "cb_delete";
	spec.title			= "Delete";
	spec.headline		= headline.c_str();
	spec.items			= items;
	spec.total			= u32(items.size());
	spec.consequence	= consequence.empty() ? nullptr : consequence.c_str();
	spec.action			= "Delete";
	spec.destructive	= true;
	const xr_vector<SEntry> doomed = m_Selection;
	const xr_string project = EditorProject::Root();
	XUI::Confirm(spec, [doomed, project](XUI::Answer answer)
	{
		// the answer comes a frame later: the project it was asked about must still be the open one
		if (XUI::Answer::Yes == answer && Form && EditorProject::Active() && 0 == _stricmp(project.c_str(), EditorProject::Root()))
			Form->DeleteEntries(doomed);
	});
}

void UIContentBrowser::DeleteEntries(const xr_vector<SEntry>& entries)
{
	if (entries.empty() || !EditorProject::Active())	return;

	EditorFileOps::SReport rep;
	for (const SEntry& e : entries)
	{
		string_path abs;
		xr_sprintf	(abs, sizeof(abs), "%s\\%s", EditorProject::Root(), e.path.c_str());
		// recursive: the selection said "this", not "this if it happens to be empty"
		EditorFileOps::Delete(abs, true, rep);
	}

	ClearSelection();
	m_NeedRefresh = true;
	if (rep.ok())
		ELog.Msg(mtInformation, "Deleted %d file(s), %d folder(s) for good.", rep.files, rep.dirs);
	else
		XUI::Notice(XUI::Tone::Error, "Delete", NqUtil::Format("Deleted %d file(s); %d could not be deleted: %s", rep.files, rep.failed,
			rep.failures.empty() ? "" : rep.failures[0].reason.c_str()).c_str());
}

bool UIContentBrowser::CanPaste() const
{
	if (m_Clipboard.empty())		return false;
	if (m_Source != 0)				return false;	// destination must be writable
	if (m_ClipSource == 1)
	{
		int n = 0;
		return !!LibFilesFor(m_ClipCategory, n);
	}
	return true;
}

LPCSTR UIContentBrowser::PasteBlockedReason() const
{
	if (m_Clipboard.empty())	return "The clipboard is empty.";
	if (m_Source != 0)			return "This source is read-only: paste into the project.";
	if (m_ClipSource == 1)
	{
		int n = 0;
		if (!LibFilesFor(m_ClipCategory, n))
			return "That category lives inside one shared library file: there is no file of its own to copy.";
	}
	return "";
}

void UIContentBrowser::PasteEntry(const SClip& c, LPCSTR dst_root, bool overwrite,
								  EditorFileOps::SReport& rep)
{
	// `rel` may carry folders of its own (a copied tree); the entry itself keeps
	// its leaf name, so the destination FOLDER is rel minus that leaf
	string_path dst;
	xr_strcpy	(dst, sizeof(dst), dst_root);
	{
		xr_string sub(c.rel);
		if (LPCSTR cut = strrchr(sub.c_str(), '\\'))
		{
			sub.erase(size_t(cut - sub.c_str()));
			xr_strcat(dst, sizeof(dst), "\\");
			xr_strcat(dst, sizeof(dst), sub.c_str());
		}
	}

	switch (m_ClipSource)
	{
	case 0:
		{
			// project -> project: real paths on both sides, and Copy/Move walk a
			// folder themselves - which is why project folders are not expanded
			string_path src;
			xr_sprintf	(src, sizeof(src), "%s\\%s", EditorProject::Root(), c.src.c_str());
			if (m_ClipCut)	EditorFileOps::Move(src, dst, true, overwrite, rep);
			else			EditorFileOps::Copy(src, dst, true, overwrite, rep);
		}
		break;

	case 2:
		// the linked game install: a gamedata-relative path that may live inside
		// an archive, which EditorFileOps::Copy knows how to read
		EditorFileOps::Copy(c.src.c_str(), dst, false, overwrite, rep);
		break;

	default:
		{
			// the SDK library: a reference, resolved through the fs.ltx aliases
			xr_string err;
			const int n = CopyLibraryItem(m_ClipCategory, c.src.c_str(), dst, overwrite, err);
			if (!err.empty())
			{
				++rep.failed;
				if (rep.failures.size() < 64)
				{
					EditorFileOps::SFailure f;
					f.path		= c.src;
					f.reason	= err;
					rep.failures.push_back(f);
				}
			}
			rep.files += n;
		}
		break;
	}
}

int UIContentBrowser::PasteClipboardInto(LPCSTR dst_root, bool overwrite, xr_string& err)
{
	err = "";

	EditorFileOps::SReport rep;
	for (u32 i = 0; i < m_Clipboard.size(); ++i)
		PasteEntry(m_Clipboard[i], dst_root, overwrite, rep);

	// a cut is spent once it has been pasted, like everywhere else
	if (m_ClipCut)	{ m_Clipboard.clear(); m_ClipCut = false; }

	m_NeedRefresh = true;
	if (!rep.ok())
	{
		char head[96];
		sprintf_s(head, sizeof(head), "%d failed, first: ", rep.failed);
		err  = head;
		err += rep.failures.empty() ? "" : rep.failures[0].reason.c_str();
	}
	return rep.files;
}

void UIContentBrowser::ClipboardPaste()
{
	if (!CanPaste()) return;

	// Straight into the folder you are looking at - that is what a paste means.
	string_path dst_root;
	xr_sprintf	(dst_root, sizeof(dst_root), "%s\\%s", EditorProject::Root(), m_CurFolder.c_str());

	// A paste has always overwritten whatever was in the way, without a word.
	// The clipboard entries carry the path they will take below dst_root, so
	// the collisions are known BEFORE anything is written - ask about those.
	XUI::Strings clashes;
	for (u32 i = 0; i < m_Clipboard.size(); ++i)
	{
		string_path t;
		xr_sprintf(t, sizeof(t), "%s\\%s", dst_root, m_Clipboard[i].rel.c_str());
		if (INVALID_FILE_ATTRIBUTES != ::GetFileAttributesA(t))
			clashes.push_back(m_Clipboard[i].rel);
	}

	const xr_string target = dst_root;
	auto paste = [target]
	{
		if (!Form || !Form->CanPaste())	return;
		xr_string err;
		const int files = Form->PasteClipboardInto(target.c_str(), true, err);
		if (err.empty())	ELog.Msg(mtInformation, "Pasted %d file(s).", files);
		else				XUI::Notice(XUI::Tone::Error, "Paste", NqUtil::Format("Pasted %d file(s); %s", files, err.c_str()).c_str());
	};
	if (clashes.empty())
	{
		paste();
		return;
	}

	// -nodlg answers No, so an unattended run never silently replaces files
	const xr_string where = m_CurFolder.empty() ? xr_string("the project's root") : m_CurFolder;
	const xr_string headline = NqUtil::Format("%u item(s) already exist in %s and will be replaced:", u32(clashes.size()), where.c_str());
	XUI::ConfirmSpec spec;
	spec.id				= "cb_replace";
	spec.title			= "Replace";
	spec.headline		= headline.c_str();
	spec.items			= clashes;
	spec.total			= u32(clashes.size());
	spec.consequence	= "What they replace is gone for good - this cannot be undone.";
	spec.action			= "Replace";
	spec.destructive	= true;
	XUI::Confirm(spec, [paste](XUI::Answer answer)
	{
		if (XUI::Answer::Yes == answer)
			paste();
	});
}

void UIContentBrowser::HandleShortcuts()
{
	// only when this panel owns the keyboard, or Ctrl+C in the viewport would
	// end up copying assets
	// the scene's own bindings stay away from a focused browser (XUI::Shell::RegisterKeyOwner)
	if (!ImGui::IsWindowFocused(ImGuiFocusedFlags_RootAndChildWindows)) return;
	if (ImGui::IsAnyItemActive()) return;
	// a text box had the keys up to this frame: the Enter that closed the rename box opens nothing
	const ImGuiIO& io = ImGui::GetIO();
	if (io.WantTextInput) return;

	if (io.KeyAlt)
	{
		xr_string err;
		if (ImGui::IsKeyPressed(ImGuiKey_LeftArrow, false)) GoBack(err);
		if (ImGui::IsKeyPressed(ImGuiKey_RightArrow, false)) GoForward(err);
		if (ImGui::IsKeyPressed(ImGuiKey_UpArrow, false)) GoUp(err);
		if (ImGui::IsKeyPressed(ImGuiKey_Home, false)) GoHome(err);
	}

	// Delete carries no modifier, and F2 renames whatever single entry is
	// selected - both belong outside the Ctrl gate below
	if (ImGui::IsKeyPressed(ImGuiKey_Delete, false))	DeleteSelection();
	if (ImGui::IsKeyPressed(ImGuiKey_F2, false) && 1 == m_Selection.size())
		BeginRename(m_Selection[0].path.c_str());
	if ((ImGui::IsKeyPressed(ImGuiKey_Enter, false) || ImGui::IsKeyPressed(ImGuiKey_KeypadEnter, false)) &&
		1 == m_Selection.size() && !io.KeyCtrl && !io.KeyAlt)
		OpenEntry(m_Selection[0]);
	if (ImGui::IsKeyPressed(ImGuiKey_F5, false))		m_NeedRefresh = true;

	if (!io.KeyCtrl) return;

	if (ImGui::IsKeyPressed(ImGuiKey_C, false))	ClipboardCopy(false);
	if (ImGui::IsKeyPressed(ImGuiKey_X, false))	ClipboardCopy(true);
	if (ImGui::IsKeyPressed(ImGuiKey_V, false))	ClipboardPaste();
	if (ImGui::IsKeyPressed(ImGuiKey_A, false))
		SelectAllViewEntries();
}

void UIContentBrowser::DrawGridContextMenu()
{
	// MCP ui_popup cb_grid: the menu a right click on empty space puts up
	if (XUI::Shell::TakePopup("cb_grid"))
		XUI::Shell::OpenPopupAt("cb_grid_ctx", ImVec2(ImGui::GetWindowPos().x + XUI::Pt(16.f), ImGui::GetWindowPos().y + XUI::Pt(16.f)));
	// "Window" variant: fires on empty space, not over a tile
	if (!ImGui::BeginPopupContextWindow("cb_grid_ctx", ImGuiPopupFlags_MouseButtonRight | ImGuiPopupFlags_NoOpenOverItems))
		return;

	// Authoring lives here too, on the empty space of the writable source - the
	// same place a file manager puts it - with the Add menu's items.
	if (m_Source == 0)
	{
		DrawAddMenu();
		ImGui::Separator();
	}

	const bool can = CanPaste();
	if (XUI::MenuItem("Paste", "Ctrl+V", false, can))
		ClipboardPaste();
	if (!can)
		XUI::Tip(PasteBlockedReason());

	ImGui::Separator();
	if (XUI::MenuItem("Select All", "Ctrl+A"))
		SelectAllViewEntries();
	if (ImGui::MenuItem("Clear Selection", nullptr, false, !m_Selection.empty()))
		ClearSelection();
	if (XUI::MenuItem("Refresh", "F5"))
		m_NeedRefresh = true;

	ImGui::EndPopup();
}

LPCSTR UIContentBrowser::AddBlocked() const
{
	if (IsReadOnlySource())
		return "Content is added to the project: switch the source to Project.";
	if (!EditorProject::Active())
		return "Open a project to add content to it.";
	return nullptr;
}

// The Add menu: a folder or one of the editor's own assets, in the folder shown - the project only
void UIContentBrowser::DrawAddMenu()
{
	LPCSTR blocked = AddBlocked();
	const bool writable = !blocked;
	struct SItem { LPCSTR label; LPCSTR tip; void (UIContentBrowser::*create)(); };
	static const SItem kItems[] =
	{
		{ "New Folder",			"A folder in the folder shown.",											&UIContentBrowser::CreateFolder },
		{ "Quest",				"A quest graph (.nqasset).",												&UIContentBrowser::CreateQuest },
		{ "Behaviour Graph",	"What an NPC does, step by step (.behasset).",							&UIContentBrowser::CreateBehaviour },
		{ "Character",			"One of the module's own NPCs (.npcasset).",								&UIContentBrowser::CreateCharacter },
		{ "PDA Interface",		"The module's own PDA screens: pages, buttons, menus, the live map (.pdaasset).", &UIContentBrowser::CreatePda },
	};
	for (u32 i = 0; i < _countof(kItems); ++i)
	{
		if (1 == i)
			ImGui::Separator();
		if (ImGui::MenuItem(kItems[i].label, nullptr, false, writable))
			(this->*kItems[i].create)();
		XUI::Tip(writable ? kItems[i].tip : blocked);
	}
}

// `dst_dir` empty means "wherever fs.ltx implies", which is what MCP wants when
// no destination was given; a paste passes the folder the user is looking at.
int UIContentBrowser::CopyLibraryItem(u32 category, LPCSTR ref, LPCSTR dst_dir,
									  bool overwrite, xr_string& err)
{
	err = "";

	int n = 0;
	const SLibFile* files = LibFilesFor(category, n);
	if (!files || !n)
	{
		err = "this category has no file of its own - it lives inside a shared library file";
		return 0;
	}

	string_path base;
	xr_strcpy	(base, sizeof(base), ref);
	// library references come without an extension, but a search result or a
	// hand-typed name may carry one - strip it so the probes below own the ext
	if (char* dot = strrchr(base, '.'))
		if (!strchr(dot, '\\')) *dot = 0;

	EditorFileOps::SReport rep;
	int hits = 0;
	for (int i = 0; i < n; ++i)
	{
		string_path rel;
		xr_sprintf	(rel, sizeof(rel), "%s%s", base, files[i].ext);

		if (dst_dir && dst_dir[0])
		{
			string_path src;
			FS.update_path	(src, files[i].alias, rel);
			if (INVALID_FILE_ATTRIBUTES == ::GetFileAttributesA(src))	continue;
			++hits;
			EditorFileOps::Copy(src, dst_dir, false, overwrite, rep);
		}
		else if (EditorFileOps::CopyLibraryFile(files[i].alias, rel, overwrite, rep))
			++hits;
	}

	if (!hits)									err = "no file behind this reference";
	else if (!rep.ok() && !rep.failures.empty())	err = rep.failures[0].reason;
	return rep.files;
}

int UIContentBrowser::CopyRefsToProject(u32 category, const xr_vector<xr_string>& names,
										bool overwrite, xr_string& err)
{
	err = "";

	int files = 0, failed = 0;
	xr_string last_err;
	for (u32 i = 0; i < names.size(); ++i)
	{
		xr_string one;
		if (m_Source == 2)
		{
			// the game install keeps its own copier: the source may be a file
			// inside an archive, which only that one can read
			string_path target = {};
			bool existed = false;
			if (EditorGameContent::CopyToProject(names[i].c_str(), overwrite, target, existed, one))
				++files;
			else if (existed && !overwrite)
				one = "already exists (pass overwrite)";
			if (!one.empty())	{ ++failed; last_err = one; }
		}
		else
		{
			// no destination: mirror into the layout fs.ltx implies
			files += CopyLibraryItem(category, names[i].c_str(), 0, overwrite, one);
			if (!one.empty())	{ ++failed; last_err = one; }
		}
	}

	if (failed)
	{
		char head[128];
		sprintf_s(head, sizeof(head), "%d item(s) failed, first: ", failed);
		err  = head;
		err += last_err;
	}
	return files;
}

//------------------------------------------------------------------------------
// folder authoring (project source only)
//------------------------------------------------------------------------------
void UIContentBrowser::CreateFolder()
{
	if (m_Source != 0 || !EditorProject::Active())	return;

	// The project root is a content folder like any other: a module keeps
	// whatever layout its author wants ([vfs] publishes it), so a folder can be
	// born anywhere - including next to gamedata.

	// first free "New Folder", "New Folder 2", ... exactly like a file manager
	xr_string rel;
	string_path probe;
	for (int n = 1; n <= 999; ++n)
	{
		char leaf[64];
		if (1 == n)	xr_strcpy(leaf, sizeof(leaf), "New Folder");
		else		sprintf_s(leaf, "New Folder %d", n);

		if (m_CurFolder.empty())	sprintf_s(probe, "%s\\%s", EditorProject::Root(), leaf);
		else						sprintf_s(probe, "%s\\%s\\%s", EditorProject::Root(), m_CurFolder.c_str(), leaf);
		if (INVALID_FILE_ATTRIBUTES == ::GetFileAttributesA(probe))
		{
			rel = m_CurFolder;
			if (!rel.empty()) rel += "\\";
			rel += leaf;
			break;
		}
	}
	if (rel.empty())	{ ELog.DlgMsg(mtError, "Too many unnamed folders here."); return; }

	string_path full;
	xr_string err;
	if (!EditorFileOps::MakeDir(rel.c_str(), full, err))
	{
		ELog.DlgMsg(mtError, "Cannot create the folder: %s", err.c_str());
		return;
	}

	// the tile has to exist before it can be renamed, and only a refresh puts it
	// in the tree - an empty folder has no items to derive it from
	Refresh			();
	ClearSelection	();
	BeginRename		(rel.c_str());
}

void UIContentBrowser::CreatePda()
{
	if (m_Source != 0 || !EditorProject::Active())	return;
	string_path folder, made;
	if (m_CurFolder.empty())	xr_strcpy(folder, sizeof(folder), EditorProject::Root());
	else						sprintf_s(folder, "%s\\%s", EditorProject::Root(), m_CurFolder.c_str());
	xr_string err;
	if (!EditorPda::CreateIn(folder, made, err))
	{
		ELog.DlgMsg(mtError, "Cannot create the PDA interface: %s", err.c_str());
		return;
	}
	xr_string rel = m_CurFolder;
	if (!rel.empty()) rel += "\\";
	rel += NqUtil::BaseName(made);
	Refresh			();
	ClearSelection	();
	BeginRename		(rel.c_str());
}

void UIContentBrowser::CreateCharacter()
{
	if (m_Source != 0 || !EditorProject::Active())	return;

	// first free NewCharacter.npcasset, NewCharacter2.npcasset, ... here
	xr_string rel;
	string_path probe;
	for (int n = 1; n <= 999; ++n)
	{
		char leaf[64];
		if (1 == n)	xr_strcpy(leaf, sizeof(leaf), "NewCharacter.npcasset");
		else		sprintf_s(leaf, "NewCharacter%d.npcasset", n);
		if (m_CurFolder.empty())	sprintf_s(probe, "%s\\%s", EditorProject::Root(), leaf);
		else						sprintf_s(probe, "%s\\%s\\%s", EditorProject::Root(), m_CurFolder.c_str(), leaf);
		if (INVALID_FILE_ATTRIBUTES == ::GetFileAttributesA(probe))
		{
			rel = m_CurFolder;
			if (!rel.empty()) rel += "\\";
			rel += leaf;
			break;
		}
	}
	if (rel.empty())	{ ELog.DlgMsg(mtError, "Too many unnamed characters here."); return; }

	// the id follows the file name, the way a quest's does
	xr_string base = NqUtil::BaseName(probe);
	const size_t dot = base.find_last_of('.');
	if (dot != xr_string::npos)	base.erase(dot);

	xr_string err;
	if (!NqLua::WriteFile(probe, EditorCharacters::Template(base.c_str()), err))
	{
		ELog.DlgMsg(mtError, "Cannot create the character: %s", err.c_str());
		return;
	}
	Refresh			();
	ClearSelection	();
	BeginRename		(rel.c_str());
}

void UIContentBrowser::CreateBehaviour()
{
	if (m_Source != 0 || !EditorProject::Active())	return;

	xr_string rel;
	string_path probe;
	for (int n = 1; n <= 999; ++n)
	{
		char leaf[64];
		if (1 == n)	xr_strcpy(leaf, sizeof(leaf), "NewBehaviour.behasset");
		else		sprintf_s(leaf, "NewBehaviour%d.behasset", n);
		if (m_CurFolder.empty())	sprintf_s(probe, "%s\\%s", EditorProject::Root(), leaf);
		else						sprintf_s(probe, "%s\\%s\\%s", EditorProject::Root(), m_CurFolder.c_str(), leaf);
		if (INVALID_FILE_ATTRIBUTES == ::GetFileAttributesA(probe))
		{
			rel = m_CurFolder;
			if (!rel.empty()) rel += "\\";
			rel += leaf;
			break;
		}
	}
	if (rel.empty())	{ ELog.DlgMsg(mtError, "Too many unnamed behaviour graphs here."); return; }

	xr_string err;
	if (!NqExport::NewBehaviourTemplate(probe, err))
	{
		ELog.DlgMsg(mtError, "Cannot create the behaviour graph: %s", err.c_str());
		return;
	}
	Refresh			();
	ClearSelection	();
	BeginRename		(rel.c_str());
}

void UIContentBrowser::CreateQuest()
{
	if (m_Source != 0 || !EditorProject::Active())	return;

	// first free NewQuest.nqasset, NewQuest2.nqasset, ... in the open folder
	xr_string rel;
	string_path probe;
	for (int n = 1; n <= 999; ++n)
	{
		char leaf[64];
		if (1 == n)	xr_strcpy(leaf, sizeof(leaf), "NewQuest.nqasset");
		else		sprintf_s(leaf, "NewQuest%d.nqasset", n);
		if (m_CurFolder.empty())	sprintf_s(probe, "%s\\%s", EditorProject::Root(), leaf);
		else						sprintf_s(probe, "%s\\%s\\%s", EditorProject::Root(), m_CurFolder.c_str(), leaf);
		if (INVALID_FILE_ATTRIBUTES == ::GetFileAttributesA(probe))
		{
			rel = m_CurFolder;
			if (!rel.empty()) rel += "\\";
			rel += leaf;
			break;
		}
	}
	if (rel.empty())	{ ELog.DlgMsg(mtError, "Too many unnamed quests here."); return; }

	xr_string err;
	if (!NqExport::NewQuestTemplate(probe, err))
	{
		ELog.DlgMsg(mtError, "Cannot create the quest: %s", err.c_str());
		return;
	}
	Refresh			();
	ClearSelection	();
	BeginRename		(rel.c_str());
}

bool UIContentBrowser::CreateAsset(LPCSTR kind, xr_string& made, xr_string& err)
{
	made.clear();
	err = "";
	if (!kind || !kind[0])		{ err = "kind must be quest, behaviour, character or pda"; return false; }
	if (!EditorProject::Active())	{ err = "no active project"; return false; }
	Show();
	if (!Form)					{ err = "content browser unavailable"; return false; }
	if (Form->m_Source != 0)	{ err = "only the project source can be written to"; return false; }
	// the listing is current whether or not a panel or the drawer drew it this frame
	Form->EnsureListing();

	// the menu items are the same three calls; they pick the first free name in
	// the open folder and refresh, so the answer is whatever appeared
	xr_vector<xr_string> before;
	for (u32 i = 0; i < Form->m_Items.size(); ++i)	before.push_back(xr_string(Form->m_Items[i].name.c_str()));

	if		(0 == _stricmp(kind, "quest"))		Form->CreateQuest();
	else if (0 == _stricmp(kind, "behaviour"))	Form->CreateBehaviour();
	else if (0 == _stricmp(kind, "character"))	Form->CreateCharacter();
	else if (0 == _stricmp(kind, "pda"))		Form->CreatePda();
	else	{ err = "kind must be quest, behaviour, character or pda"; return false; }

	// the inline rename box belongs to the menu, not to a programmatic call
	Form->m_Rename.clear();
	Form->m_RenameFocus = false;

	for (u32 i = 0; i < Form->m_Items.size(); ++i)
	{
		bool had = false;
		for (u32 k = 0; k < before.size() && !had; ++k)
			had = (0 == _stricmp(before[k].c_str(), Form->m_Items[i].name.c_str()));
		if (!had)	{ made = Form->m_Items[i].name.c_str(); return true; }
	}
	err = "nothing was created";
	return false;
}

void UIContentBrowser::BeginRename(LPCSTR path)
{
	if (m_Source != 0 || !path || !path[0])	return;

	m_Rename		= path;
	m_RenameFocus	= true;

	LPCSTR leaf = strrchr(path, '\\');
	xr_strcpy(m_RenameBuf, sizeof(m_RenameBuf), leaf ? leaf + 1 : path);

	// A folder is its whole name. A file is renamed by its name: one of the editor's
	// own assets does not even show its extension (it is the kind, and stays -
	// EditorFileOps::Rename), any other file shows it and keeps it out of the selection.
	m_RenameSelect = int(xr_strlen(m_RenameBuf));
	string_path disk;
	if (xr_strlen(EditorProject::Root()) + xr_strlen(path) + 2 < sizeof(disk))
	{
		sprintf_s(disk, "%s\\%s", EditorProject::Root(), path);
		const DWORD attr = ::GetFileAttributesA(disk);
		if (INVALID_FILE_ATTRIBUTES != attr && !(attr & FILE_ATTRIBUTE_DIRECTORY))
		{
			if (LPCSTR own = EditorAssetIcons::OwnExtension(m_RenameBuf))
			{
				m_RenameBuf[own - m_RenameBuf] = 0;
				m_RenameSelect = int(xr_strlen(m_RenameBuf));
			}
			else if (LPCSTR dot = strrchr(m_RenameBuf, '.'))
			{
				if (dot != m_RenameBuf) m_RenameSelect = int(dot - m_RenameBuf);
			}
		}
	}
	m_RenameStem = m_RenameSelect;
}

void UIContentBrowser::CommitRename()
{
	if (m_Rename.empty())	return;

	const xr_string was = m_Rename;
	m_Rename.clear();

	// EditorFileOps::Rename is the one implementation, shared with MCP content_rename:
	// the leaf rules, the quest extension, and a change of case alone being a rename
	string_path renamed;
	xr_string err;
	if (!EditorFileOps::Rename(was.c_str(), m_RenameBuf, renamed, err))
	{
		ELog.DlgMsg(mtError, "Cannot rename '%s': %s.", was.c_str(), err.c_str());
		return;
	}
	LPCSTR s = strrchr(renamed, '\\');
	s = s ? s + 1 : renamed;
	LPCSTR old_leaf = strrchr(was.c_str(), '\\');
	old_leaf = old_leaf ? old_leaf + 1 : was.c_str();
	if (0 == xr_strcmp(s, old_leaf))	return;		// unchanged, nothing to do

	xr_string parent(was);
	if (LPCSTR cut = strrchr(parent.c_str(), '\\'))
		parent.erase(size_t(cut - parent.c_str()));
	else
		parent.clear();

	// keep the user where they were looking: the open folder may BE the renamed
	// one, or sit under it
	xr_string now(parent);
	if (!now.empty()) now += "\\";
	now += s;
	if (PathIsAncestor(was, m_CurFolder))
		m_CurFolder = now + (m_CurFolder.c_str() + was.size());

	auto rewrite_location = [&](SLocation& location)
	{
		if (location.source != 0 || !PathIsAncestor(was, location.folder)) return;
		location.folder = now + (location.folder.c_str() + was.size());
	};
	for (u32 i = 0; i < m_BackHistory.size(); ++i) rewrite_location(m_BackHistory[i]);
	for (u32 i = 0; i < m_ForwardHistory.size(); ++i) rewrite_location(m_ForwardHistory[i]);
	for (u32 i = 0; i < s_Favorites.size(); ++i) rewrite_location(s_Favorites[i]);

	m_NeedRefresh = true;
}

//------------------------------------------------------------------------------
// the panel: the toolbar, the Sources pane beside the tiles, the status strip
//------------------------------------------------------------------------------
// The view settings live in the layout (the .ini), read with it at the first frame: the tiles keep
// their size and the pane its place across sessions.
void UIContentBrowser::LoadView()
{
	if (m_ViewLoaded) return;
	m_ViewLoaded	= true;
	// 0 (or nothing stored) = no size picked: the tiles fit the host
	const auto picked = [](LPCSTR key)
	{
		const float size = XUI::Layout::Get(key, 0.f);
		return size > 0.f ? clampr(size, EditorUiContract::kTileMin, EditorUiContract::kTileMax) : 0.f;
	};
	m_TileSize[kHostDrawer]	= picked("content_browser.tile_size");
	m_TileSize[kHostPanel]	= picked("content_browser.tile_size_panel");
	// the docked panel lists: the Wide layout's strip holds a row of tiles and a half
	m_ListView[kHostDrawer]	= XUI::Layout::Get("content_browser.list", 0.f) != 0.f;
	m_ListView[kHostPanel]	= XUI::Layout::Get("content_browser.list_panel", 1.f) != 0.f;
	m_SourcesPane	= XUI::Layout::Get("content_browser.sources_pane", 1.f) != 0.f;
}

void UIContentBrowser::StoreView()
{
	m_ViewLoaded = true;
	XUI::Layout::Set("content_browser.tile_size", m_TileSize[kHostDrawer]);
	XUI::Layout::Set("content_browser.tile_size_panel", m_TileSize[kHostPanel]);
	XUI::Layout::Set("content_browser.list", m_ListView[kHostDrawer] ? 1.f : 0.f);
	XUI::Layout::Set("content_browser.list_panel", m_ListView[kHostPanel] ? 1.f : 0.f);
	XUI::Layout::Set("content_browser.sources_pane", m_SourcesPane ? 1.f : 0.f);
}

LPCSTR UIContentBrowser::RootLabel() const
{
	switch (m_Source)
	{
	case 0:		return "Project";
	case 2:		return "Game";
	default:	return kCategories[m_Category].caption;
	}
}

// "Game / textures / ui": where a favorite points, the way a tooltip names it
static xr_string FavoriteLabel(const UIContentBrowser::SLocation& location)
{
	xr_string label = 0 == location.source ? "Project" : 2 == location.source ? "Game" : "Editor";
	if (1 == location.source)
	{
		label += " / ";
		label += UIContentBrowser::CategoryName(location.category);
	}
	for (LPCSTR p = location.folder.c_str(); *p;)
	{
		LPCSTR cut = strchr(p, '\\');
		LPCSTR end = cut ? cut : p + xr_strlen(p);
		label += " / ";
		label.append(p, end);
		p = cut ? cut + 1 : end;
	}
	return label;
}

// 59155 -> "59,155": counts in the status strip are read, not deciphered
static xr_string Thousands(int n)
{
	char digits[16];
	sprintf_s(digits, "%d", n);
	xr_string out;
	const int length = int(xr_strlen(digits));
	for (int i = 0; i < length; ++i)
	{
		if (i && '-' != digits[i - 1] && 0 == (length - i) % 3)
			out += ',';
		out += digits[i];
	}
	return out;
}

float UIContentBrowser::BreadcrumbWidth() const
{
	const float pad = XUI::Pt(12.f), sep = XUI::Pt(16.f);
	float w = ImGui::CalcTextSize(RootLabel()).x + pad;
	for (LPCSTR p = m_CurFolder.c_str(); *p;)
	{
		LPCSTR cut = strchr(p, '\\');
		LPCSTR end = cut ? cut : p + xr_strlen(p);
		w += sep + ImGui::CalcTextSize(p, end).x + pad;
		p = cut ? cut + 1 : end;
	}
	return w;
}

void UIContentBrowser::DrawBreadcrumbs(float width)
{
	struct SCrumb
	{
		xr_string label, path;
		float w;
	};
	const float pad = XUI::Pt(12.f), sep = XUI::Pt(16.f), fold_w = XUI::Pt(28.f);
	xr_vector<SCrumb> crumbs;
	crumbs.push_back({ RootLabel(), "", ImGui::CalcTextSize(RootLabel()).x + pad });
	xr_string path;
	for (LPCSTR p = m_CurFolder.c_str(); *p;)
	{
		LPCSTR cut = strchr(p, '\\');
		LPCSTR end = cut ? cut : p + xr_strlen(p);
		const xr_string part = Span(p, end);
		if (!path.empty()) path += "\\";
		path += part;
		crumbs.push_back({ part, path, ImGui::CalcTextSize(part.c_str()).x + pad });
		p = cut ? cut + 1 : end;
	}

	// the leading crumbs fold into a menu until the rest fits; the folder shown always stays
	const int n = int(crumbs.size());
	float total = 0.f;
	for (int i = 0; i < n; ++i)
		total += crumbs[i].w + (i ? sep : 0.f);
	int first = 0;
	while (total > width && first < n - 1)
	{
		total -= crumbs[first].w + sep;
		if (!first) total += fold_w + sep;
		++first;
	}

	ImDrawList* dl = ImGui::GetWindowDrawList();
	const ImVec2 origin = ImGui::GetCursorScreenPos();
	const float h = ImGui::GetFrameHeight();
	const float right = origin.x + width;
	const float r = XUI::Pt(4.f);
	float x = origin.x;
	auto chevron = [&]
	{
		XUI::DrawIcon(dl, XUI::Icon::ChevronRight, ImVec2(PxRound(x + XUI::Pt(2.f)), PxRound(origin.y + (h - XUI::Pt(12.f)) * .5f)), 12.f,
			ThemeCol(Token::TextMuted));
		x += sep;
	};
	auto paint = [&](const ImVec2& a, float w, LPCSTR text, ImU32 col)
	{
		dl->AddText(ImVec2(PxRound(a.x + (w - ImGui::CalcTextSize(text).x) * .5f), PxRound(a.y + (h - ImGui::GetFontSize()) * .5f)), col, text);
	};

	ImGui::PushID("cb_crumbs");
	int go = -1;
	if (first > 0)
	{
		const ImVec2 a(x, origin.y);
		ImGui::SetCursorScreenPos(a);
		if (ImGui::InvisibleButton("##fold", ImVec2(fold_w, h)))
			XUI::Shell::OpenPopupAt("##folded", ImVec2(a.x, a.y + h + XUI::Pt(2.f)));
		const bool hovered = ImGui::IsItemHovered();
		if (hovered)
			dl->AddRectFilled(a, ImVec2(a.x + fold_w, a.y + h), ThemeCol(ImGui::IsItemActive() ? Token::Pressed : Token::Hover), r);
		paint(a, fold_w, kEllipsis, ThemeCol(hovered ? Token::Text : Token::TextSecondary));
		XUI::Tip("The folders above");
		x += fold_w;
		chevron();
	}
	for (int i = first; i < n; ++i)
	{
		if (i > first)
			chevron();
		const bool last = i == n - 1;
		// the folder shown gives up its middle before the row gives up anything else
		const float w = last ? _max(XUI::Pt(24.f), _min(crumbs[i].w, right - x)) : crumbs[i].w;
		const ImVec2 a(x, origin.y);
		ImGui::PushID(i);
		ImGui::SetCursorScreenPos(a);
		const bool pressed = ImGui::InvisibleButton("##crumb", ImVec2(w, h));
		const bool hovered = ImGui::IsItemHovered();
		if (hovered && !last)
			dl->AddRectFilled(a, ImVec2(a.x + w, a.y + h), ThemeCol(ImGui::IsItemActive() ? Token::Pressed : Token::Hover), r);
		xr_string shown;
		const bool cut = XUI::EllipsizeText(crumbs[i].label.c_str(), w - pad, XUI::Clip::End, shown);
		paint(a, w, cut ? shown.c_str() : crumbs[i].label.c_str(), ThemeCol(last || hovered ? Token::Text : Token::TextSecondary));
		if (cut)
		{
			XUI::Tip(crumbs[i].label.c_str());
			XUI::AuditTruncated(crumbs[i].label.c_str(), shown.c_str(), a, ImVec2(a.x + w, a.y + h));
		}
		if (pressed && !last)
			go = i;
		ImGui::PopID();
		x += w;
	}
	if (ImGui::BeginPopup("##folded"))
	{
		for (int i = 0; i < first; ++i)
		{
			ImGui::PushID(i);
			if (ImGui::MenuItem(crumbs[i].label.c_str()))
				go = i;
			ImGui::PopID();
		}
		ImGui::EndPopup();
	}
	ImGui::PopID();

	if (go >= 0)
	{
		xr_string err;
		ApplyLocation(SLocation(m_Source, int(m_Category), crumbs[go].path.c_str()), true, err);
	}
}

void UIContentBrowser::DrawViewMenu()
{
	ImGui::SeparatorText("View As");
	if (ImGui::MenuItem("Tiles", nullptr, !m_ListView[m_Host]))
	{
		m_ListView[m_Host] = false;
		StoreView();
	}
	XUI::Tip("A picture and a two-line name for each entry.");
	if (ImGui::MenuItem("List", nullptr, m_ListView[m_Host]))
	{
		m_ListView[m_Host] = true;
		StoreView();
	}
	XUI::Tip("One line for each entry, in columns: the most entries in a short panel.");

	ImGui::SeparatorText("Tile Size");
	const bool fit = m_TileSize[m_Host] <= 0.f;
	if (ImGui::MenuItem("Fit to Height", nullptr, fit))
	{
		// off keeps the size shown, now as the one picked
		m_TileSize[m_Host] = fit ? ShownTile(m_Host) : 0.f;
		m_ListView[m_Host] = false;
		StoreView();
	}
	{
		char tip[160];
		sprintf_s(tip, "The largest tiles, up to %.0f pt, that let the grid show two rows. Picking a size turns it off.",
			kHostPanel == m_Host ? EditorUiContract::kTileDefaultPanel : EditorUiContract::kTileDefault);
		XUI::Tip(tip);
	}
	float size = ShownTile(m_Host);
	ImGui::SetNextItemWidth(XUI::Em(12.f));
	if (ImGui::SliderFloat("##cb_tile", &size, EditorUiContract::kTileMin, EditorUiContract::kTileMax, "%.0f pt"))
	{
		m_TileSize[m_Host] = size;
		m_ListView[m_Host] = false;
		StoreView();
	}
	XUI::Tip("The side of a tile; the panel and the Content Drawer keep one each. Ctrl+wheel over the tiles changes it "
		"too, and goes to the list under the smallest.");

	ImGui::SeparatorText("Source");
	static const LPCSTR kSources[] = { "Project", "Editor", "Game" };
	for (int i = 0; i < 3; ++i)
		if (ImGui::MenuItem(kSources[i], nullptr, m_Source == i))
			SwitchSource(i);
	if (1 == m_Source && ImGui::BeginMenu("Category"))
	{
		for (int i = 0; i < kCategoryCount; ++i)
			if (ImGui::MenuItem(kCategories[i].caption, nullptr, m_Category == u32(i)))
			{
				xr_string err;
				ApplyLocation(SLocation(1, i, ""), true, err);
			}
		ImGui::EndMenu();
	}

	ImGui::Separator();
	if (ImGui::MenuItem("Sources Pane", nullptr, m_SourcesPane))
	{
		m_SourcesPane = !m_SourcesPane;
		StoreView();
	}
	XUI::Tip(m_SourcesFit ? "The source, the favorites and the folder tree beside the tiles."
		: "Shown while the browser is at least 480 pt wide.");
	if (XUI::MenuItem("Refresh", "F5"))
		m_NeedRefresh = true;
}

void UIContentBrowser::DrawFavoritesMenu()
{
	SyncFavoriteScopes();
	if (s_Favorites.empty())
	{
		ImGui::PushTextWrapPos(ImGui::GetCursorPosX() + XUI::Em(18.f));
		XUI::StatusText(XUI::Tone::Muted, "No favorites yet. The star beside this menu keeps the folder shown.");
		ImGui::PopTextWrapPos();
		return;
	}
	const SLocation here = CurrentLocation();
	const float row_w = XUI::Em(22.f);
	const float h = XUI::Pt(24.f);
	for (u32 i = 0; i < s_Favorites.size(); ++i)
	{
		const SLocation favorite = s_Favorites[i];
		const xr_string label = FavoriteLabel(favorite);
		ImGui::PushID(int(i));
		const ImVec2 at = ImGui::GetCursorScreenPos();
		const bool open = ImGui::Selectable("##favorite", SameLocation(favorite, here), ImGuiSelectableFlags_None, ImVec2(row_w, h));
		// a path: the start gives way, the folder's own name stays
		xr_string shown;
		const bool cut = XUI::EllipsizeText(label.c_str(), row_w - XUI::Pt(8.f), XUI::Clip::Start, shown);
		ImGui::GetWindowDrawList()->AddText(ImVec2(PxRound(at.x + XUI::Pt(4.f)), PxRound(at.y + (h - ImGui::GetFontSize()) * .5f)),
			ThemeCol(Token::Text), cut ? shown.c_str() : label.c_str());
		if (cut)
		{
			XUI::Tip(label.c_str());
			XUI::AuditTruncated(label.c_str(), shown.c_str(), at, ImVec2(at.x + row_w, at.y + h));
		}
		ImGui::SameLine(0.f, XUI::Pt(2.f));
		const bool remove = XUI::IconButton("##remove", XUI::Icon::Close, "Remove from Favorites");
		ImGui::PopID();
		if (open)
		{
			xr_string err;
			if (!ApplyLocation(favorite, true, err))
				XUI::Toast(XUI::Tone::Warning, ("The favorite cannot be opened: " + err).c_str());
			break;
		}
		if (remove)
		{
			s_Favorites.erase(s_Favorites.begin() + i);
			break;
		}
	}
}

void UIContentBrowser::DrawSources()
{
	static const LPCSTR kSources[] = { "Project", "Editor", "Game" };
	int source = m_Source;
	ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
	if (XUI::Segmented("##cb_source", &source, kSources, 3))
		SwitchSource(source);
	// the Editor source is the shared library, one category at a time
	if (1 == m_Source)
	{
		ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
		if (ImGui::BeginCombo("##cb_category", kCategories[m_Category].caption))
		{
			for (int i = 0; i < kCategoryCount; ++i)
				if (ImGui::Selectable(kCategories[i].caption, m_Category == u32(i)))
				{
					xr_string err;
					ApplyLocation(SLocation(1, i, ""), true, err);
				}
			ImGui::EndCombo();
		}
		XUI::Tip("The category of the editor's library listed here.");
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));

	// The two section headers cost 56 pt. A pane that cannot hold six tree rows beside them - the Wide layout's strip -
	// keeps that room for the tree: its favorites, when there are any, go straight above it with a rule between.
	// Measured here, outside the scrolling child, whose own measure grows as it scrolls.
	const bool sections = ImGui::GetContentRegionAvail().y >= XUI::Pt(28.f * 2.f + 22.f * 6.f);

	if (ImGui::BeginChild("##cb_sources_scroll", ImVec2(0.f, 0.f)))
	{
		// rows touch: their 22 pt is the rhythm, tree nodes included
		ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(ImGui::GetStyle().ItemSpacing.x, 0.f));
		ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(XUI::Pt(4.f), XUI::Pt(3.f)));
		SyncFavoriteScopes();
		const auto favorite_rows = [this]
		{
			const SLocation here = CurrentLocation();
			for (u32 i = 0; i < s_Favorites.size(); ++i)
			{
				const SLocation favorite = s_Favorites[i];
				LPCSTR leaf = strrchr(favorite.folder.c_str(), '\\');
				xr_string name = leaf ? leaf + 1 : favorite.folder.c_str();
				if (name.empty())
					name = 0 == favorite.source ? "Project" : 2 == favorite.source ? "Game" : CategoryName(favorite.category);
				const xr_string where = FavoriteLabel(favorite);
				char id[16];
				sprintf_s(id, "fav%u", i);
				bool remove = false;
				XUI::NavRowSpec row;
				row.icon = XUI::Icon::StarFilled;
				row.tip = where.c_str();
				row.action = XUI::Icon::Close;
				row.action_tip = "Remove from Favorites";
				if (XUI::NavRow(id, name.c_str(), SameLocation(favorite, here), row, &remove))
				{
					xr_string err;
					if (!ApplyLocation(favorite, true, err))
						XUI::Toast(XUI::Tone::Warning, ("The favorite cannot be opened: " + err).c_str());
				}
				if (remove)
				{
					s_Favorites.erase(s_Favorites.begin() + i);
					return;
				}
			}
		};
		if (sections)
		{
			char count[16];
			sprintf_s(count, "%u", u32(s_Favorites.size()));
			// closed while there is nothing in it, until the author opens it: the tree needs the room more
			if (XUI::Section("Favorites", !s_Favorites.empty(), s_Favorites.empty() ? nullptr : count))
			{
				if (s_Favorites.empty())
					XUI::StatusText(XUI::Tone::Muted, "The star on the toolbar keeps the folder shown here.");
				favorite_rows();
				XUI::EndSection();
			}
			if (XUI::Section("Folders", true))
			{
				DrawFolder(m_Root);
				XUI::EndSection();
			}
		}
		else
		{
			if (!s_Favorites.empty())
			{
				favorite_rows();
				ImGui::Dummy(ImVec2(0.f, XUI::Pt(3.f)));
				ImGui::Separator();
				ImGui::Dummy(ImVec2(0.f, XUI::Pt(3.f)));
			}
			DrawFolder(m_Root);
		}
		ImGui::PopStyleVar(2);
	}
	ImGui::EndChild();
}

// The view's own test, per selected entry rather than per tile: a selection is rarely the size of a folder.
int UIContentBrowser::SelectedTiles() const
{
	if (m_Selection.empty())
		return 0;
	const bool searching = SearchActive(m_Search);
	const size_t below = m_CurFolder.empty() ? 0 : m_CurFolder.size() + 1;
	int count = 0;
	for (const SEntry& e : m_Selection)
	{
		LPCSTR path = e.path.c_str();
		// strictly under the folder shown: the folder itself (picked in the tree) is no tile of it
		if (e.path.size() <= below ||
			(below && (0 != _strnicmp(path, m_CurFolder.c_str(), below - 1) || path[below - 1] != '\\')))
			continue;
		// a search lists everything below that matches, a folder only what it holds itself
		if (searching ? XUI::Match(m_Search, path + below) : !strchr(path + below, '\\'))
			++count;
	}
	return count;
}

void UIContentBrowser::DrawStatusStrip()
{
	const float h = XUI::Pt(24.f);
	const ImVec2 p = ImGui::GetCursorScreenPos();
	const float w = _max(1.f, ImGui::GetContentRegionAvail().x);
	ImGui::GetWindowDrawList()->AddLine(p, ImVec2(p.x + w, p.y), ThemeCol(Token::BorderSubtle), _max(1.f, XUI::Pt(1.f)));

	XUI::FontScope font(XUI::Font::Small);
	// the folder shown - or what the search found in it - never the whole source
	const int shown = m_ViewFolders + m_ViewFiles;
	xr_string text = Thousands(shown);
	text += SearchActive(m_Search) ? (1 == shown ? " result" : " results") : (1 == shown ? " item" : " items");
	LPCSTR dot = "  \xC2\xB7  ";
	if (const int selected = SelectedTiles())
	{
		text += dot;
		text += Thousands(selected);
		text += " selected";
	}
	if (!m_Clipboard.empty())
	{
		text += dot;
		text += Thousands(int(m_Clipboard.size()));
		text += m_ClipCut ? " cut" : " on the clipboard";
	}
	const bool read_only = IsReadOnlySource();
	const float badge_w = read_only ? ImGui::CalcTextSize("Read only").x + XUI::Pt(12.f) : 0.f;
	const float line_y = PxRound(p.y + (h - ImGui::GetTextLineHeight()) * .5f);
	ImGui::SetCursorScreenPos(ImVec2(p.x + XUI::Pt(2.f), line_y));
	ImGui::PushStyleColor(ImGuiCol_Text, ThemeCol(Token::TextSecondary));
	XUI::TextEllipsis(text.c_str(), _max(0.f, w - badge_w - XUI::Pt(14.f)));
	ImGui::PopStyleColor();
	if (read_only)
	{
		ImGui::SetCursorScreenPos(ImVec2(p.x + w - badge_w, line_y));
		XUI::Badge("Read only");
		XUI::Tip("Browse, preview and copy out into the project: nothing in this source is changed.");
	}
	ImGui::SetCursorScreenPos(p);
	ImGui::Dummy(ImVec2(w, h));
}

// [Link Game...] of the Game source's empty state: the same check and link as the project's own page
void UIContentBrowser::LinkGameFolder()
{
	char picked[MAX_PATH * 3] = {};
	// the native folder picker; the Status line under Link Game says what the folder must hold
	if (!XUI::BrowsePath(picked, sizeof(picked), true))
		return;
	if (!EditorProject::LinkGame(picked))
	{
		XUI::Notice(XUI::Tone::Error, "Link Game", EditorProject::LinkError());
		return;
	}
	m_DarfReady		= false;
	m_DarfStatus	= "Reading the linked game.";
	m_NeedRefresh	= true;
}

void UIContentBrowser::DrawToolbar()
{
	xr_string err;
	// The breadcrumb takes what the rest of the row leaves, measured the way the toolbar lays it out at full
	// labels; the toolbar folds labels and items away first when even its minimum does not fit.
	const float row_w = ImGui::GetContentRegionAvail().x;
	auto menu_w = [](LPCSTR label) { return XUI::Pt(8.f + 16.f + 6.f + 10.f + 4.f + 12.f) + ImGui::CalcTextSize(label).x; };
	const float button = XUI::Pt(24.f), in_group = XUI::Pt(2.f), between = XUI::Pt(12.f);
	const float separator = XUI::Pt(12.f) + _max(1.f, XUI::Pt(1.f));
	const float others = menu_w("Add") + separator + button * 3.f + in_group * 2.f + separator + in_group + between +
		XUI::Em(10.f) + between + menu_w("View") + between + button + in_group + menu_w("Favorites") + XUI::Pt(8.f);
	const float crumbs_w = _max(XUI::Em(6.f), _min(BreadcrumbWidth(), row_w - others));

	XUI::Toolbar tb("cb_toolbar", XUI::ToolbarPanel);
	tb.Group(3);
	// Add works in the open project's own source only: elsewhere it is off and says why, instead of a menu of
	// items that are all off
	LPCSTR add_off = AddBlocked();
	if (tb.Menu("cb_add", XUI::Icon::Plus, "Add", add_off ? add_off : "A folder or an asset in the folder shown", nullptr,
		!add_off))
	{
		DrawAddMenu();
		ImGui::EndPopup();
	}
	// an MCP request is taken either way; off, there is nothing to put up
	else if (XUI::Shell::TakePopup("cb_add") && !add_off)
	{
		// MCP ui_popup cb_add: the menu comes up under its button, where a click puts it
		ImGui::PushID("cb_add");
		XUI::Shell::OpenPopupAt("##menu", ImVec2(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y + XUI::Pt(2.f)));
		ImGui::PopID();
	}
	tb.Separator();
	tb.Group(4);
	if (tb.Action("cb_back", XUI::Icon::Back, "", "Back (Alt+Left)", !m_BackHistory.empty()))
		GoBack(err);
	if (tb.Action("cb_forward", XUI::Icon::Forward, "", "Forward (Alt+Right)", !m_ForwardHistory.empty()))
		GoForward(err);
	if (tb.Action("cb_up", XUI::Icon::Up, "", "Parent Folder (Alt+Up)", !m_CurFolder.empty()))
		GoUp(err);
	tb.Separator();
	tb.Group(5);
	if (tb.Widget("cb_breadcrumb", crumbs_w / XUI::Dpi()))
		DrawBreadcrumbs(crumbs_w);
	tb.Spring();
	tb.Group(2);
	if (tb.Search(m_Search, sizeof(m_Search), "Search", 10.f))
		InvalidateView();
	tb.Group(1);
	if (tb.Menu("cb_view", XUI::Icon::ViewOptions, "View", "Tiles or list, tile size, source and the Sources pane"))
	{
		DrawViewMenu();
		ImGui::EndPopup();
	}
	else if (XUI::Shell::TakePopup("cb_view"))
	{
		ImGui::PushID("cb_view");
		XUI::Shell::OpenPopupAt("##menu", ImVec2(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y + XUI::Pt(2.f)));
		ImGui::PopID();
	}
	tb.Group(0);
	const int favorite = FindFavorite(CurrentLocation());
	const bool can_favorite = favorite >= 0 || m_Source != 0 || EditorProject::Active();
	if (tb.Action("cb_star", favorite >= 0 ? XUI::Icon::StarFilled : XUI::Icon::Star, "",
		favorite >= 0 ? "Remove from Favorites" : "Add to Favorites", can_favorite, favorite >= 0))
	{
		if (favorite >= 0)
			s_Favorites.erase(s_Favorites.begin() + favorite);
		else if (!AddFavorite(CurrentLocation(), err))
			XUI::Toast(XUI::Tone::Warning, ("The folder cannot be a favorite: " + err).c_str());
	}
	if (tb.Menu("cb_favorites", XUI::Icon::Bookmark, "Favorites", "Favorite folders"))
	{
		DrawFavoritesMenu();
		ImGui::EndPopup();
	}
	else if (XUI::Shell::TakePopup("cb_favorites"))
	{
		ImGui::PushID("cb_favorites");
		XUI::Shell::OpenPopupAt("##menu", ImVec2(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y + XUI::Pt(2.f)));
		ImGui::PopID();
	}
}

void UIContentBrowser::Draw()
{
	ImGui::PushStyleVar(ImGuiStyleVar_WindowMinSize, ImVec2(XUI::Pt(360.f), XUI::Pt(150.f)));
	// The toolbar and the status strip are bands of their own, flush with the edges as in the drawer: the
	// strip's height goes to the tiles
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(ImGui::GetStyle().WindowPadding.x, 0.f));
	const bool visible = XUI::BeginPanel(PanelName(), &bOpen);
	ImGui::PopStyleVar(2);
	m_PanelFrame = ImGui::GetFrameCount();
	m_RaiseFrames = visible ? 0 : _max(0, m_RaiseFrames - 1);
	m_Host = kHostPanel;
	if (visible)
	{
		if (XUI::Shell::OpenedDrawer() == XUI::Shell::DrawerContent)
			XUI::EmptyState(XUI::Icon::Drawer, "In the Content Drawer", "Close the drawer to browse here.");
		else
			DrawBody();
	}
	XUI::EndPanel();
}

void UIContentBrowser::DrawBody()
{
	m_Tick++;
	m_ThumbsThisFrame = 0;
	if (!ImGui::IsMouseDragging(ImGuiMouseButton_Left)) m_Dragged.clear();
	LoadView();
	EnsureListing();

	DrawToolbar();

	// the Sources pane beside the tiles, the status strip under both
	const ImGuiStyle& st = ImGui::GetStyle();
	const float status_h = XUI::Pt(24.f);
	const float width = ImGui::GetContentRegionAvail().x;
	// whole pixels: the panel has no padding under the status strip, and a fraction over would scroll it
	const float body_h = _max(XUI::Pt(48.f), floorf(ImGui::GetContentRegionAvail().y - status_h - st.ItemSpacing.y));
	m_SourcesFit	= width >= XUI::Pt(480.f);
	m_SourcesShown	= m_SourcesPane && m_SourcesFit;

	if (ImGui::BeginChild("##cb_panes", ImVec2(0.f, body_h), ImGuiChildFlags_None,
		ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse))
	{
		if (m_SourcesShown)
		{
			float side = 220.f;
			XUI::Splitter("content_browser.sources", true, &side, 160.f, 200.f, 220.f);
			ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0.f, XUI::Pt(4.f)));
			const bool pane = ImGui::BeginChild("##cb_sources", ImVec2(XUI::Pt(side), 0.f), ImGuiChildFlags_AlwaysUseWindowPadding,
				ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse);
			ImGui::PopStyleVar();
			if (pane)
				DrawSources();
			ImGui::EndChild();
			ImGui::SameLine();
		}

		// what the tiles of a host with no size picked fit (ShownTile): measured here, outside the scrolling grid
		m_GridHeight[m_Host] = _max(0.f, ImGui::GetContentRegionAvail().y - XUI::Pt(kGridPadY) * 2.f) / XUI::Dpi();

		ImGui::PushStyleColor(ImGuiCol_ChildBg, ThemeCol(Token::Inset));
		// a tile keeps 4 pt of its own around the square: 4 more above and below is the grid's 8
		ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(8.f), XUI::Pt(kGridPadY)));
		// under Ctrl the wheel sizes the tiles instead of scrolling them
		const bool tiles = ImGui::BeginChild("##cb_tiles", ImVec2(0.f, 0.f), ImGuiChildFlags_AlwaysUseWindowPadding,
			ImGui::GetIO().KeyCtrl ? ImGuiWindowFlags_NoScrollWithMouse : ImGuiWindowFlags_None);
		ImGui::PopStyleVar();
		ImGui::PopStyleColor();
		if (tiles)
		{
			// IsGameSource, not IsReadOnlySource: the SDK library is read-only too but
			// has nothing to do with the game install being linked
			if (IsGameSource() && !m_DarfReady)
			{
				if (XUI::EmptyState(XUI::Icon::GameFolder,
					EditorProject::GameLinked() ? "Game content unavailable" : "No game linked",
					m_DarfStatus.empty() ? "Link the game install to browse its content." : m_DarfStatus.c_str(),
					"Link Game..."))
					LinkGameFolder();
			}
			else
				DrawTiles();
		}
		ImGui::EndChild();
	}
	ImGui::EndChild();

	DrawStatusStrip();

	// both panes have had their frame at it
	m_ScrollToSelection = false;

	// Panel-wide, so the tree answers Del and Ctrl+C exactly like the grid.
	// Runs after both panes: Ctrl+A needs the complete view cache from the grid.
	HandleShortcuts();
}
