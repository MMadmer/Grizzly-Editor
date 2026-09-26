#include "stdafx.h"
#include "EditorAssetIcons.h"
#include "ui_main.h"
#include "Nq/NqUtil.h"
#include "../../XrEUI/XrUITheme.h"
#include "../../XrEUI/XrUIIcons.h"

namespace
{
//------------------------------------------------------------------------------
// what a file is
//
// The extension answers it for a quest (.nqasset) and for a behaviour graph
// (.behasset) - that is the point of giving each its own. Only an `.npcasset`
// has to be looked INTO, to tell a template from a character; that is in the
// first line or two of a canonical file, so a kilobyte is plenty - and the
// answer is kept, because a tile that stats its file every frame is exactly the
// kind of thing the startup profile was written to stop.
//------------------------------------------------------------------------------
u32											s_Generation = 1;
xr_flat_hash_map<xr_string, u32>			s_CachedGen;
xr_flat_hash_map<xr_string, u8>				s_CachedKind;

// `key = "value"` or `key = true`, as the canonical writer spells them and as a
// hand-written file is likely to
bool HeadSays(const char* text, u32 len, LPCSTR key, LPCSTR value)
{
	const u32 klen = (u32)xr_strlen(key);
	for (u32 i = 0; i + klen < len; ++i)
	{
		if (i && (isalnum((u8)text[i - 1]) || text[i - 1] == '_'))	continue;
		if (0 != _strnicmp(text + i, key, klen))					continue;
		u32 j = i + klen;
		if (j < len && (isalnum((u8)text[j]) || text[j] == '_'))		continue;
		while (j < len && (text[j] == ' ' || text[j] == '\t'))		++j;
		if (j >= len || text[j] != '=')								continue;
		++j;
		while (j < len && (text[j] == ' ' || text[j] == '\t' || text[j] == '"'))	++j;
		const u32 vlen = (u32)xr_strlen(value);
		if (j + vlen <= len && 0 == _strnicmp(text + j, value, vlen))	return true;
	}
	return false;
}

bool ReadHead(LPCSTR path, char* buf, u32 cap, u32& got)
{
	got = 0;
	HANDLE h = ::CreateFileA(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, NULL,
		OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN, NULL);
	if (INVALID_HANDLE_VALUE == h) return false;
	DWORD read = 0;
	const BOOL ok = ::ReadFile(h, buf, cap - 1, &read, NULL);
	::CloseHandle(h);
	if (!ok) return false;
	buf[read] = 0;
	got = (u32)read;
	return true;
}
} // namespace

//------------------------------------------------------------------------------
LPCSTR EditorAssetIcons::OwnExtension(LPCSTR name)
{
	LPCSTR ext = name ? strrchr(name, '.') : 0;
	// a dot in a folder of the path is not an extension, and an extension alone is not a name
	if (!ext || ext == name || strpbrk(ext, "\\/") || '\\' == ext[-1] || '/' == ext[-1])	return 0;
	if (0 == _stricmp(ext, ".nqasset") || 0 == _stricmp(ext, ".behasset") || 0 == _stricmp(ext, ".npcasset") ||
		0 == _stricmp(ext, ".pdaasset"))
		return ext;
	return 0;
}

EditorAssetIcons::EKind EditorAssetIcons::KindOf(LPCSTR path)
{
	if (!path || !path[0]) return kNone;

	LPCSTR ext = strrchr(path, '.');
	if (!ext) return kNone;
	// no file is touched for these two: the name is the whole answer
	if (0 == _stricmp(ext, ".nqasset"))		return kQuest;
	if (0 == _stricmp(ext, ".behasset"))	return kBehaviour;
	if (0 == _stricmp(ext, ".pdaasset"))	return kPda;
	if (0 != _stricmp(ext, ".npcasset"))	return kNone;

	xr_string key = path;
	xr_strlwr(&key[0]);

	// cached, unless the generation moved on
	auto gen = s_CachedGen.find(key);
	if (gen != s_CachedGen.end() && gen->second == s_Generation)
	{
		auto k = s_CachedKind.find(key);
		if (k != s_CachedKind.end()) return (EKind)k->second;
	}

	EKind kind = kCharacter;
	char head[1024];
	u32	 got = 0;
	if (ReadHead(path, head, sizeof(head), got) && got && HeadSays(head, got, "abstract", "true"))
		kind = kCharacterTemplate;

	s_CachedGen[key]	= s_Generation;
	s_CachedKind[key]	= (u8)kind;
	return kind;
}

void EditorAssetIcons::Invalidate()
{
	++s_Generation;
	// the maps are kept: the entries are overwritten as the tiles come back, and
	// a browser refresh must not turn into a burst of allocations
	if (s_CachedGen.size() > 4096)
	{
		s_CachedGen.clear();
		s_CachedKind.clear();
	}
}

LPCSTR EditorAssetIcons::Name(EKind kind)
{
	switch (kind)
	{
	case kQuest:				return "quest";
	case kBehaviour:			return "behaviour graph";
	case kCharacter:			return "character";
	case kCharacterTemplate:	return "character template";
	case kPda:					return "PDA interface";
	default:					return "";
	}
}

XUI::Icon EditorAssetIcons::IconFor(EKind kind)
{
	switch (kind)
	{
	case kQuest:				return XUI::Icon::Quest;
	case kBehaviour:			return XUI::Icon::Behaviour;
	case kCharacter:			return XUI::Icon::Character;
	case kCharacterTemplate:	return XUI::Icon::CharacterTemplate;
	case kPda:					return XUI::Icon::PdaInterface;
	default:					return XUI::Icon::None;
	}
}

void EditorAssetIcons::Draw(EKind kind, float x0, float y0, float x1, float y1)
{
	Draw(ImGui::GetWindowDrawList(), kind, x0, y0, x1, y1);
}

void EditorAssetIcons::Draw(ImDrawList* draw_list, EKind kind, float x0, float y0, float x1, float y1, u32 color)
{
	const XUI::Icon icon = IconFor(kind);
	if (!draw_list || icon == XUI::Icon::None) return;

	// the 14 drawn units of the glyph take 58% of the shorter side, centred - up to the 48 pt a
	// tile's icon stops at (docs/UI_DESIGN.md, section 3): past it the stroke only gets heavier
	const float	w		= x1 - x0;
	const float	h		= y1 - y0;
	const float	side	= std::min((w < h ? w : h) * 0.58f * 16.f / 14.f, XUI::Pt(48.f));
	if (side < 4.f) return;
	const float	ox		= x0 + (w - side) * 0.5f;
	const float	oy		= y0 + (h - side) * 0.5f;
	XUI::DrawIconRect(draw_list, icon, ImVec2(ox, oy), ImVec2(ox + side, oy + side),
		color ? color : XFinedTheme::U32(XFinedTheme::ColorToken::Text));
}

void EditorAssetIcons::DrawFor(LPCSTR path, float x0, float y0, float x1, float y1)
{
	Draw(KindOf(path), x0, y0, x1, y1);
}
