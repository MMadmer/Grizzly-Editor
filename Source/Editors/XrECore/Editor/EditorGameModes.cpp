#include "stdafx.h"
#include "EditorGameModes.h"
#include "EditorGameContent.h"
#include "EditorProject.h"
#include "GrizzlyMCP.h"

namespace
{
static xr_vector<EditorGameModes::SMode>	s_Modes;
static bool									s_Scanned	= false;
static xr_string							s_Error;

// The game's text and UI files are cp1251; ImGui draws UTF-8. Only the
// captions travel through here - ids and keys are ascii by construction.
xr_string Cp1251ToUtf8(LPCSTR src)
{
	xr_string out;
	if (!src) return out;
	const int wide_len = ::MultiByteToWideChar(1251, 0, src, -1, NULL, 0);
	if (wide_len <= 0) { out = src; return out; }
	xr_vector<wchar_t> wide(wide_len);
	::MultiByteToWideChar(1251, 0, src, -1, wide.data(), wide_len);
	const int utf_len = ::WideCharToMultiByte(CP_UTF8, 0, wide.data(), -1, NULL, 0, NULL, NULL);
	if (utf_len <= 0) { out = src; return out; }
	xr_vector<char> utf(utf_len);
	::WideCharToMultiByte(CP_UTF8, 0, wide.data(), -1, utf.data(), utf_len, NULL, NULL);
	out = utf.data();
	return out;
}

// mod.ltx is written by this editor, so its captions are already UTF-8 - but a
// hand-written manifest may well be cp1251 like every other file in the game.
// Valid UTF-8 is left alone; anything else is read as cp1251.
xr_string CaptionFromManifest(LPCSTR src)
{
	if (!src) return xr_string();
	const int ok = ::MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, src, -1, NULL, 0);
	return ok > 0 ? xr_string(src) : Cp1251ToUtf8(src);
}

// whole item of the linked game as a string; empty when it is not there
xr_string ReadGameText(LPCSTR rel)
{
	xr_string text;
	const int idx = EditorGameContent::Find(rel);
	if (idx < 0) return text;
	u32 size = 0;
	u8* bytes = EditorGameContent::ReadBytes(idx, size);
	if (bytes && size)
	{
		text.assign((const char*)bytes, (const char*)bytes + size);
		text += '\0';
	}
	EditorGameContent::FreeBytes(bytes);
	return text;
}

int AttrInt(const xr_string& text, size_t tag_at, LPCSTR attr, int def)
{
	const size_t close = text.find('>', tag_at);
	if (close == xr_string::npos) return def;
	char pat[32];
	sprintf_s(pat, "%s=\"", attr);
	const size_t len = xr_strlen(pat);
	// the name has to START where it is found: "x=" also lives inside "max=",
	// and "height=" inside "item_height=", and both are real X-Ray attributes
	for (size_t at = text.find(pat, tag_at); at != xr_string::npos && at < close;
		 at = text.find(pat, at + len))
	{
		const char before = at ? text[at - 1] : ' ';
		if (before == ' ' || before == '\t' || before == '\r' || before == '\n')
			return atoi(text.c_str() + at + len);
	}
	return def;
}

// new_game_metro_mode -> metro, new_game_good_loot -> good_loot
xr_string NormalizeId(const xr_string& config_key)
{
	xr_string id = config_key;
	if (0 == _strnicmp(id.c_str(), "new_game_", 9))	id.erase(0, 9);
	const size_t n = id.size();
	if (n > 5 && 0 == _stricmp(id.c_str() + n - 5, "_mode"))	id.erase(n - 5);
	return id;
}

// Lua block comments hide whole InitCheck calls: Dead Air's story and azazel
// checkboxes sit inside --[[ ... --]] with the call itself unprefixed, so a
// per-line check alone would call them live.
bool InsideBlockComment(const xr_string& script, size_t at)
{
	bool block = false;
	for (size_t i = 0; i + 1 < at; ++i)
	{
		if (block)
		{
			if (script[i] == ']' && script[i + 1] == ']') { block = false; ++i; }
			continue;
		}
		if (script[i] != '-' || script[i + 1] != '-') continue;
		// --[[ opens a block; anything else is a line comment, skip to its end
		if (i + 3 < at && script[i + 2] == '[' && script[i + 3] == '[') { block = true; i += 3; }
		else
		{
			const size_t eol = script.find('\n', i);
			if (eol == xr_string::npos || eol >= at) return block;
			i = eol;
		}
	}
	return block;
}

// A checkbox is only real when the script creates it: Dead Air ships several
// that are commented out (azazel, story, and DAR2's km), and offering those
// would target a mode the player can never pick.
bool CheckboxLive(const xr_string& script, const xr_string& node)
{
	xr_string needle = "main_dialog:";
	needle += node;
	size_t at = 0;
	while ((at = script.find(needle, at)) != xr_string::npos)
	{
		// walk back to the line start and see whether it is commented out
		size_t bol = script.rfind('\n', at);
		bol = (bol == xr_string::npos) ? 0 : bol + 1;
		xr_string line = script.substr(bol, at - bol);
		size_t first = line.find_first_not_of(" \t");
		const bool commented = (first != xr_string::npos) && 0 == strncmp(line.c_str() + first, "--", 2);
		if (!commented && line.find("InitCheck") != xr_string::npos && !InsideBlockComment(script, at))
			return true;
		at += needle.size();
	}
	return false;
}

// st_cap_check_metro_mode -> "Dead Air Metro", from whatever language the
// install actually ships. Scans every text xml once, on demand.
void ResolveCaptions(xr_vector<EditorGameModes::SMode>& modes)
{
	// caption still holds the string id at this point; a parallel flag records
	// what has been resolved, because a resolved caption is arbitrary text and
	// cannot be told from an id by looking at it
	xr_vector<xr_string> string_ids(modes.size());
	bool any = false;
	for (u32 i = 0; i < modes.size(); ++i)
	{
		string_ids[i] = modes[i].caption;
		modes[i].caption.clear();
		if (!string_ids[i].empty()) any = true;
	}
	if (!any)
	{
		for (u32 i = 0; i < modes.size(); ++i) modes[i].caption = modes[i].id;
		return;
	}

	const int total = EditorGameContent::Count();
	for (int i = 0; i < total; ++i)
	{
		const EditorGameContent::SItem* it = EditorGameContent::Get(i);
		if (!it) continue;
		if (0 != _strnicmp(it->path.c_str(), "configs\\text\\", 13))	continue;
		if (it->path.size() < 4 || 0 != _stricmp(it->path.c_str() + it->path.size() - 4, ".xml")) continue;

		const xr_string text = ReadGameText(it->path.c_str());
		if (text.empty()) continue;

		for (u32 m = 0; m < modes.size(); ++m)
		{
			EditorGameModes::SMode& mode = modes[m];
			if (string_ids[m].empty() || !mode.caption.empty()) continue;	// nothing to look up, or done
			xr_string pat = "id=\"";
			pat += string_ids[m];
			pat += "\"";
			const size_t at = text.find(pat);
			if (at == xr_string::npos) continue;
			const size_t open = text.find("<text>", at);
			const size_t end  = (open == xr_string::npos) ? xr_string::npos : text.find("</text>", open);
			if (open == xr_string::npos || end == xr_string::npos) continue;
			xr_string raw = text.substr(open + 6, end - open - 6);
			mode.caption = Cp1251ToUtf8(raw.c_str());
		}
	}

	// a string id nobody defines stays readable rather than becoming noise
	for (u32 m = 0; m < modes.size(); ++m)
		if (modes[m].caption.empty())
			modes[m].caption = modes[m].id;
}

// [provides_mode] of every XMS module installed in the linked game
// Modules live in <game>\modules\; <game>\mods\ is the folder JSGME manages and
// still holds anything installed before the split, so both are read.
static const char* kModuleRoots[] = { "modules", "mods" };

void ScanModuleRoot(LPCSTR game, LPCSTR root, xr_vector<EditorGameModes::SMode>& out)
{
	string_path mask;
	sprintf_s(mask, "%s\\%s\\*", game, root);
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (INVALID_HANDLE_VALUE == h) return;
	do
	{
		if (!(fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY))	continue;
		if (!strcmp(fd.cFileName, ".") || !strcmp(fd.cFileName, ".."))	continue;

		string_path manifest;
		sprintf_s(manifest, "%s\\%s\\%s\\mod.ltx", game, root, fd.cFileName);
		HANDLE f = ::CreateFileA(manifest, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING,
								 FILE_ATTRIBUTE_NORMAL, NULL);
		if (INVALID_HANDLE_VALUE == f) continue;
		LARGE_INTEGER size;
		xr_string text;
		if (::GetFileSizeEx(f, &size) && size.QuadPart > 0 && size.QuadPart < (1 << 20))
		{
			xr_vector<char> buf(size_t(size.QuadPart) + 1, 0);
			DWORD read = 0;
			if (::ReadFile(f, buf.data(), DWORD(size.QuadPart), &read, NULL)) text = buf.data();
		}
		::CloseHandle(f);
		if (text.empty()) continue;

		// [provides_mode] carries id/title pairs; id opens a new entry
		const size_t sect = text.find("[provides_mode]");
		if (sect == xr_string::npos) continue;
		size_t pos = sect;
		EditorGameModes::SMode pending;
		while (pos < text.size())
		{
			const size_t eol = text.find('\n', pos);
			xr_string line = text.substr(pos, (eol == xr_string::npos ? text.size() : eol) - pos);
			pos = (eol == xr_string::npos) ? text.size() : eol + 1;
			if (line.find('[') != xr_string::npos && line.find("[provides_mode]") == xr_string::npos) break;
			const size_t eq = line.find('=');
			if (eq == xr_string::npos) continue;
			xr_string key = line.substr(0, eq), val = line.substr(eq + 1);
			while (!key.empty() && (key.back() == ' ' || key.back() == '\t')) key.pop_back();
			while (!val.empty() && (val.front() == ' ' || val.front() == '\t')) val.erase(val.begin());
			while (!val.empty() && (val.back() == ' ' || val.back() == '\t' || val.back() == '\r')) val.pop_back();
			if (0 == _stricmp(key.c_str(), "id"))
			{
				if (!pending.id.empty()) out.push_back(pending);
				pending			= EditorGameModes::SMode();
				pending.id		= val;
				pending.caption	= val;
				pending.campaign= true;
				pending.source	= fd.cFileName;
			}
			else if (0 == _stricmp(key.c_str(), "title") && !pending.id.empty())
				pending.caption = CaptionFromManifest(val.c_str());
		}
		if (!pending.id.empty()) out.push_back(pending);
	} while (::FindNextFileA(h, &fd));
	::FindClose(h);
}

// [provides_mode] of every XMS module installed in the linked game
void ScanInstalledModules(xr_vector<EditorGameModes::SMode>& out)
{
	LPCSTR game = EditorProject::GameRoot();
	if (!game || !game[0]) return;

	const u32 was = (u32)out.size();
	for (int r = 0; r < 2; ++r)
	{
		xr_vector<EditorGameModes::SMode> found;
		ScanModuleRoot(game, kModuleRoots[r], found);
		// the same module in both roots declares its mode once
		for (u32 i = 0; i < found.size(); ++i)
		{
			bool dup = false;
			for (u32 j = was; j < out.size() && !dup; ++j)
				dup = (0 == _stricmp(out[j].id.c_str(), found[i].id.c_str()));
			if (!dup) out.push_back(found[i]);
		}
	}
}
} // namespace

void EditorGameModes::Invalidate()
{
	s_Scanned = false;
	s_Modes.clear();
	s_Error.clear();
}

bool EditorGameModes::Scan(xr_string& err)
{
	err.clear();
	if (s_Scanned) { err = s_Error; return s_Error.empty(); }
	s_Scanned = true;
	s_Modes.clear();

	xr_string mount_err;
	if (!EditorGameContent::EnsureMounted(mount_err))
	{
		s_Error = mount_err.empty() ? xr_string("no game linked") : mount_err;
		err = s_Error;
		return false;
	}

	// the 16:9 layout is what the game actually uses; the 4:3 one is a fallback
	xr_string ui = ReadGameText("configs\\ui\\ui_mm_faction_select_16.xml");
	if (ui.empty()) ui = ReadGameText("configs\\ui\\ui_mm_faction_select.xml");
	const xr_string script = ReadGameText("scripts\\ui_mm_faction_select.script");
	if (ui.empty())
	{
		s_Error = "the linked game has no ui_mm_faction_select xml - cannot tell modes from options";
		err = s_Error;
		ScanInstalledModules(s_Modes);
		return false;
	}

	// Column headers decide which kind a checkbox is. Without them (a stock
	// 0.98b install) nothing here is a campaign - the stock checkboxes are all
	// rule toggles, and the game itself calls them options.
	int x_mode = -1, x_option = -1;
	if (const size_t at = ui.find("<dar2_mode"); at != xr_string::npos)		x_mode   = AttrInt(ui, at, "x", -1);
	if (const size_t at = ui.find("<dar2_option"); at != xr_string::npos)	x_option = AttrInt(ui, at, "x", -1);

	size_t pos = 0;
	while ((pos = ui.find("<check_", pos)) != xr_string::npos)
	{
		const size_t name_at = pos + 1;
		size_t name_end = name_at;
		while (name_end < ui.size() && (isalnum((u8)ui[name_end]) || ui[name_end] == '_')) ++name_end;
		const xr_string node = ui.substr(name_at, name_end - name_at);
		const int x = AttrInt(ui, pos, "x", -1);
		pos = name_end;

		SMode m;
		m.source		= "game";
		// The checkbox node and the option key do not always match:
		// check_metro_mode pairs with new_game_metro_mode, but check_survival
		// pairs with new_game_survival_mode. Take whichever spelling the
		// script actually uses.
		{
			const xr_string base	= "new_game_" + node.substr(6);
			const xr_string with	= base + "_mode";
			m.config_key = (!script.empty() && script.find(with) != xr_string::npos) ? with : base;
		}
		m.id			= NormalizeId(m.config_key);
		m.available		= script.empty() ? true : CheckboxLive(script, node);

		// nearer column wins; with no headers at all nothing is a campaign
		if (x_mode >= 0 && x_option >= 0 && x >= 0)
			m.campaign = abs(x - x_mode) <= abs(x - x_option);
		else
			m.campaign = false;

		// the caption lives on the sibling label node, as a string-table id
		xr_string cap_tag = "<cap_";
		cap_tag += node;
		if (const size_t cap_at = ui.find(cap_tag); cap_at != xr_string::npos)
		{
			const size_t open = ui.find('>', cap_at);
			const size_t t_open = (open == xr_string::npos) ? xr_string::npos : ui.find('>', ui.find("<text", open));
			const size_t t_end  = (t_open == xr_string::npos) ? xr_string::npos : ui.find("</text>", t_open);
			if (t_open != xr_string::npos && t_end != xr_string::npos)
				m.caption = ui.substr(t_open + 1, t_end - t_open - 1);
		}
		s_Modes.push_back(m);
	}

	ResolveCaptions(s_Modes);

	// options are parsed only to be told apart; they never reach a caller
	xr_vector<SMode> kept;
	for (u32 i = 0; i < s_Modes.size(); ++i)
		if (s_Modes[i].campaign && s_Modes[i].available) kept.push_back(s_Modes[i]);
	s_Modes = kept;

	ScanInstalledModules(s_Modes);
	return true;
}

const xr_vector<EditorGameModes::SMode>& EditorGameModes::All()
{
	return s_Modes;
}

void EditorGameModes::McpList(xr_string& out)
{
	xr_string err;
	const bool ok = Scan(err);
	out  = ok ? "{\"ok\":true" : "{\"ok\":false";
	if (!ok) { out += ",\"error\":\""; EditorGameContent::JsonAppendPath(out, err.c_str()); out += "\""; }
	out += ",\"modes\":[";
	const xr_vector<SMode>& all = All();
	for (u32 i = 0; i < all.size(); ++i)
	{
		if (i) out += ",";
		out += "{\"id\":\"";		EditorGameContent::JsonAppendPath(out, all[i].id.c_str());
		out += "\",\"caption\":\"";	EditorGameContent::JsonAppendPath(out, all[i].caption.c_str());
		out += "\",\"key\":\"";		EditorGameContent::JsonAppendPath(out, all[i].config_key.c_str());
		out += "\",\"source\":\"";	EditorGameContent::JsonAppendPath(out, all[i].source.c_str());
		out += "\"}";
	}
	out += "]}";
}
