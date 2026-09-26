#include "stdafx.h"
#include "EditorGameContent.h"
#include "EditorGameModes.h"
#include "EditorModManifest.h"
#include "EditorUiContract.h"
#include "EditorCatalog.h"
#include "EditorCharacters.h"
#include "EditorWorld.h"
#include "EditorPda.h"
#include "EditorProject.h"
#include "XFinedMCP.h"
#include "EDX11Utils.h"
#include "ui_main.h"
#include "Nq/NqExport.h"
#include "Nq/NqDoc.h"
#include "Nq/NqUtil.h"
#include <shellapi.h>
#include <shobjidl.h>
#include <bcrypt.h>		// SHA-256 of a finished package
#include "Zlib/zip.h"	// minizip, release packages
#include "Zlib/unzip.h"	// reading a finished package back for the file index

//------------------------------------------------------------------------------
// small file/string helpers (WinAPI only, like EditorProject)
//------------------------------------------------------------------------------
static bool DirExists(const char* p)
{
	DWORD a = ::GetFileAttributesA(p);
	return a != INVALID_FILE_ATTRIBUTES && (a & FILE_ATTRIBUTE_DIRECTORY);
}

static bool FileExists(const char* p)
{
	DWORD a = ::GetFileAttributesA(p);
	return a != INVALID_FILE_ATTRIBUTES && !(a & FILE_ATTRIBUTE_DIRECTORY);
}

static void CreateDirChain(const char* folder)
{
	string_path acc = {};
	for (const char* p = folder;; ++p)
	{
		if (*p == '\\' || *p == 0)
		{
			if (acc[0] && acc[xr_strlen(acc) - 1] != ':') ::CreateDirectoryA(acc, NULL);
			if (*p == 0) break;
		}
		acc[p - folder] = *p;
		acc[p - folder + 1] = 0;
	}
}

static bool ReadTextFile(const char* path, xr_string& out)
{
	out.clear();
	HANDLE h = ::CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
	if (h == INVALID_HANDLE_VALUE) return false;
	const DWORD size = ::GetFileSize(h, NULL);
	if (size && size != INVALID_FILE_SIZE)
	{
		out.resize(size);
		DWORD rd = 0;
		if (::ReadFile(h, &out[0], size, &rd, NULL))	out.resize(rd);
		else											out.clear();
	}
	::CloseHandle(h);
	return true;
}

static bool WriteTextFile(const char* path, const char* text)
{
	HANDLE h = ::CreateFileA(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
	if (h == INVALID_HANDLE_VALUE) return false;
	DWORD wr = 0;
	const DWORD len = (DWORD)xr_strlen(text);
	const bool ok = !!::WriteFile(h, text, len, &wr, NULL) && wr == len;
	::CloseHandle(h);
	return ok;
}

// Everything the game reads as text is cp1251; ImGui hands us UTF-8. Only
// captions travel this way - ids and keys are ascii by construction. A caption
// that is not valid UTF-8 was written by hand in cp1251 already and is passed
// through: converting it would turn every Cyrillic letter into '?'.
static xr_string Utf8ToCp1251(const char* src)
{
	xr_string out;
	if (!src || !src[0]) return out;
	const int wide_len = ::MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, src, -1, NULL, 0);
	if (wide_len <= 0) { out = src; return out; }
	xr_vector<wchar_t> wide(wide_len);
	::MultiByteToWideChar(CP_UTF8, 0, src, -1, wide.data(), wide_len);
	const int ansi_len = ::WideCharToMultiByte(1251, 0, wide.data(), -1, NULL, 0, NULL, NULL);
	if (ansi_len <= 0) { out = src; return out; }
	xr_vector<char> ansi(ansi_len);
	::WideCharToMultiByte(1251, 0, wide.data(), -1, ansi.data(), ansi_len, "?", NULL);
	out = ansi.data();
	return out;
}

static bool HasNonAscii(const char* s)
{
	for (; *s; ++s)
		if ((unsigned char)*s >= 0x80) return true;
	return false;
}

// File names come out of the ANSI file APIs; a zip entry name is UTF-8.
static xr_string AnsiToUtf8(const char* src)
{
	xr_string out = src;
	const int wide_len = ::MultiByteToWideChar(CP_ACP, 0, src, -1, NULL, 0);
	if (wide_len <= 0) return out;
	xr_vector<wchar_t> wide(wide_len);
	::MultiByteToWideChar(CP_ACP, 0, src, -1, wide.data(), wide_len);
	const int utf8_len = ::WideCharToMultiByte(CP_UTF8, 0, wide.data(), -1, NULL, 0, NULL, NULL);
	if (utf8_len <= 0) return out;
	xr_vector<char> utf8(utf8_len);
	::WideCharToMultiByte(CP_UTF8, 0, wide.data(), -1, utf8.data(), utf8_len, NULL, NULL);
	out = utf8.data();
	return out;
}

static xr_string XmlEscape(const char* src)
{
	xr_string out;
	for (const char* c = src; c && *c; ++c)
	{
		switch (*c)
		{
		case '&':	out += "&amp;";		break;
		case '<':	out += "&lt;";		break;
		case '>':	out += "&gt;";		break;
		default:	out += *c;			break;
		}
	}
	return out;
}

static void Trim(xr_string& s)
{
	const size_t b = s.find_first_not_of(" \t\r\n");
	const size_t e = s.find_last_not_of(" \t\r\n");
	s = (b == xr_string::npos) ? "" : s.substr(b, e - b + 1);
}

// splits on `sep`, trims entries, drops empties; '\r' counts as '\n'
static void SplitList(xr_vector<xr_string>& dst, LPCSTR src, char sep)
{
	dst.clear();
	xr_string cur;
	for (const char* p = src;; ++p)
	{
		if (*p == sep || *p == 0 || (sep == '\n' && *p == '\r'))
		{
			Trim(cur);
			if (!cur.empty()) dst.push_back(cur);
			cur.clear();
			if (*p == 0) break;
		}
		else cur += *p;
	}
}

static void JoinList(const xr_vector<xr_string>& src, char* dst, u32 dst_size, LPCSTR sep)
{
	xr_string all;
	for (u32 i = 0; i < src.size(); ++i)
	{
		if (i) all += sep;
		all += src[i];
	}
	strncpy_s(dst, dst_size, all.c_str(), _TRUNCATE);
}

// forward slashes in, JSON-escaped double backslashes in - one clean path out
static void NormalizePath(const char* src, char* dst, u32 dst_size)
{
	u32 o = 0;
	for (const char* p = src; *p && o + 1 < dst_size; ++p)
	{
		const char c = (*p == '/') ? '\\' : *p;
		// collapse doubled separators but keep a UNC "\\server" prefix
		if (c == '\\' && o > 1 && dst[o - 1] == '\\') continue;
		dst[o++] = c;
	}
	while (o && dst[o - 1] == '\\') --o;
	dst[o] = 0;
}

//------------------------------------------------------------------------------
// json helpers
//------------------------------------------------------------------------------
static void JsonAppend(xr_string& out, LPCSTR s)
{
	for (const char* p = s; *p; ++p)
	{
		// A raw control character inside a JSON string is not JSON at all: the reader on
		// the other end gives up on the whole answer over one newline in a message.
		switch (*p)
		{
		case '"':	out += "\\\""; break;
		case '\\':	out += "\\\\"; break;
		case '\n':	out += "\\n"; break;
		case '\r':	out += "\\r"; break;
		case '\t':	out += "\\t"; break;
		default:
			if ((unsigned char)*p < 0x20)
			{
				char esc[8];
				sprintf_s(esc, "\\u%04x", (unsigned)(unsigned char)*p);
				out += esc;
			}
			else out += *p;
		}
	}
}

static void JsonAppendPath(xr_string& out, LPCSTR s)
{
	for (const char* p = s; *p; ++p) out += (*p == '\\') ? '/' : *p;
}

static void JsonAppendArray(xr_string& out, const xr_vector<xr_string>& v)
{
	out += "[";
	for (u32 i = 0; i < v.size(); ++i)
	{
		if (i) out += ",";
		out += "\"";
		JsonAppend(out, v[i].c_str());
		out += "\"";
	}
	out += "]";
}

// bare (unquoted) bool argument: {"flat":true}
static bool ArgBool(LPCSTR raw, LPCSTR field, bool def)
{
	char pat[64];
	sprintf_s(pat, "\"%s\"", field);
	const char* k = raw ? strstr(raw, pat) : 0;
	if (!k) return def;
	const char* c = strchr(k + xr_strlen(pat), ':');
	if (!c) return def;
	while (*++c == ' ') {}
	return 0 == strncmp(c, "true", 4);
}

//------------------------------------------------------------------------------
// id rules
//------------------------------------------------------------------------------
bool EditorMod::ValidateId(LPCSTR id)
{
	if (!id || !id[0]) return false;
	for (const char* p = id; *p; ++p)
	{
		const char c = *p;
		const bool ok = (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') ||
						c == '_' || c == '.' || c == '-';
		if (!ok) return false;
	}
	return true;
}

bool EditorMod::ValidateModeId(LPCSTR id)
{
	return !id || !id[0] || ValidateId(id);
}

void EditorMod::DeriveId(LPCSTR project_name, char* dst, u32 dst_size)
{
	u32 o = 0;
	for (const char* p = project_name ? project_name : ""; *p && o + 1 < dst_size; ++p)
	{
		char c = *p;
		if (c >= 'A' && c <= 'Z') c = char(c - 'A' + 'a');
		const bool ok = (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') ||
						c == '_' || c == '.' || c == '-';
		dst[o++] = ok ? c : '_';
	}
	dst[o] = 0;
	if (!o) strncpy_s(dst, dst_size, "unnamed_mod", _TRUNCATE);
}

//------------------------------------------------------------------------------
// metadata rules - the game enforces the same ones (MOD_UPDATES.md, section 1)
//------------------------------------------------------------------------------
// `min_parts`..`max_parts` dot-separated decimal numbers, 9 digits each at most
static bool DottedNumber(LPCSTR v, int min_parts, int max_parts)
{
	if (!v || !v[0]) return false;
	int parts = 1, digits = 0;
	for (const char* p = v; *p; ++p)
	{
		if (*p >= '0' && *p <= '9')
		{
			if (++digits > 9) return false;
		}
		else if (*p == '.')
		{
			if (!digits || ++parts > max_parts) return false;
			digits = 0;
		}
		else return false;
	}
	return digits > 0 && parts >= min_parts;
}

bool EditorMod::ValidateVersion(LPCSTR version)
{
	// 1.2 caps a release version at 32 characters; four 9-digit numbers would be 39
	return DottedNumber(version, 1, 4) && xr_strlen(version) <= 32;
}

static bool IsAlnum(char c)
{
	return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9');
}

bool EditorMod::ValidateWebsite(LPCSTR url)
{
	if (!url || !url[0]) return true;
	if (xr_strlen(url) > 512) return false;
	static const char kScheme[] = "https://";
	if (0 != strncmp(url, kScheme, sizeof(kScheme) - 1)) return false;

	// The host is everything up to the first '/', compared whole: "user@host",
	// "host:port" and "host.evil.com" all land inside it and fail the match.
	const char* host = url + sizeof(kScheme) - 1;
	const char* rest = strchr(host, '/');
	const size_t host_len = rest ? size_t(rest - host) : xr_strlen(host);
	static const char* kHosts[] = { "ap-pro.ru", "www.ap-pro.ru", "moddb.com", "www.moddb.com" };
	bool known = false;
	for (int i = 0; i < (int)(sizeof(kHosts) / sizeof(kHosts[0])); ++i)
		if (host_len == xr_strlen(kHosts[i]) && 0 == _strnicmp(host, kHosts[i], host_len)) known = true;
	if (!known) return false;

	for (const char* p = rest; p && *p; ++p)
		if (!IsAlnum(*p) && !strchr("-._~/?#&=%+", *p)) return false;
	return true;
}

bool EditorMod::ValidateGithubRepo(LPCSTR repo)
{
	if (!repo || !repo[0]) return true;
	const char* slash = strchr(repo, '/');
	if (!slash) return false;
	const size_t owner_len = size_t(slash - repo);
	const char* name = slash + 1;
	const size_t name_len = xr_strlen(name);
	if (owner_len < 1 || owner_len > 39 || name_len < 1 || name_len > 100) return false;
	for (const char* p = repo; p < slash; ++p)
		if (!IsAlnum(*p) && *p != '-') return false;
	// a second '/' is not in the repository charset, so "a/b/c" fails here
	for (const char* p = name; *p; ++p)
		if (!IsAlnum(*p) && *p != '-' && *p != '.' && *p != '_') return false;
	return 0 != strcmp(name, ".") && 0 != strcmp(name, "..");
}

void EditorMod::NormalizeGithubRepo(LPCSTR in, xr_string& out)
{
	out = in ? in : "";
	Trim(out);
	static const char* kPrefixes[] =
	{
		"https://github.com/", "http://github.com/", "https://www.github.com/",
		"http://www.github.com/", "github.com/", "www.github.com/",
	};
	for (int i = 0; i < (int)(sizeof(kPrefixes) / sizeof(kPrefixes[0])); ++i)
	{
		const size_t n = xr_strlen(kPrefixes[i]);
		if (0 != _strnicmp(out.c_str(), kPrefixes[i], n)) continue;
		out.erase(0, n);
		// a page URL carries more than the repository: ?tab=, #readme, /tree/main
		const size_t query = out.find_first_of("?#");
		if (query != xr_string::npos) out.erase(query);
		const size_t owner_end = out.find('/');
		const size_t repo_end = (owner_end == xr_string::npos) ? owner_end : out.find('/', owner_end + 1);
		if (repo_end != xr_string::npos) out.erase(repo_end);
		break;
	}
	// clone URLs end in .git; GitHub allows no repository name that does
	if (out.size() > 4 && 0 == _stricmp(out.c_str() + out.size() - 4, ".git"))
		out.erase(out.size() - 4);
}

//------------------------------------------------------------------------------
// mod.ltx parse / write
//------------------------------------------------------------------------------
// Offset of the ';' that starts a comment, npos when the line has none. A ';'
// between a pair of double quotes belongs to the value (MOD_UPDATES.md 1.1);
// a quote that never closes is an ordinary character.
static size_t FindComment(const xr_string& s)
{
	for (size_t i = 0; i < s.size(); ++i)
	{
		if (s[i] == ';') return i;
		if (s[i] != '"') continue;
		size_t close = i + 1;
		for (; close < s.size() && s[close] != '"'; ++close)
			if (s[close] == '\\' && close + 1 < s.size()) ++close;
		if (close < s.size()) i = close;
	}
	return xr_string::npos;
}

// "..." -> text: \n, \" and \\ are decoded, anything else stays as written.
// False (dst untouched) when `val` is not exactly one quoted string.
static bool Unquote(const xr_string& val, xr_string& dst)
{
	if (val.size() < 2 || val[0] != '"') return false;
	xr_string out;
	size_t i = 1;
	for (; i < val.size() && val[i] != '"'; ++i)
	{
		if (val[i] == '\\' && i + 1 < val.size())
		{
			const char next = val[i + 1];
			if (next == 'n' || next == '"' || next == '\\')
			{
				out += (next == 'n') ? '\n' : next;
				++i;
				continue;
			}
		}
		out += val[i];
	}
	if (i + 1 != val.size()) return false;	// never closed, or text after the closing quote
	dst = out;
	return true;
}

static xr_string Quote(const xr_string& s)
{
	xr_string out = "\"";
	for (size_t i = 0; i < s.size(); ++i)
	{
		switch (s[i])
		{
		case '\\':	out += "\\\\";	break;
		case '"':	out += "\\\"";	break;
		case '\n':	out += "\\n";	break;
		default:	out += s[i];	break;
		}
	}
	out += '"';
	return out;
}

// name and author stay bare unless the bare form would not read back as written
static bool NeedsQuotes(const xr_string& s)
{
	if (s.empty()) return false;
	if (s.find_first_of(";\"\n") != xr_string::npos) return true;
	return s[0] == ' ' || s[0] == '\t' || s[s.size() - 1] == ' ' || s[s.size() - 1] == '\t';
}

bool EditorMod::Load(LPCSTR project_root, SManifest& m)
{
	m = SManifest();
	if (!project_root || !project_root[0]) return false;
	char path[MAX_PATH];
	sprintf_s(path, "%s\\mod.ltx", project_root);
	xr_string text;
	if (!ReadTextFile(path, text)) return false;
	// UTF-8, a BOM is skipped, anything that is not valid UTF-8 is cp1251 -
	// the same reading the game gives the file
	text = NqUtil::GameTextToUtf8(text.data(), (u32)text.size());

	enum ESec { secNone, secModule, secRequires, secOrder, secConflicts, secProvides, secUpdate, secExclude, secWorld, secPda, secExtra };
	ESec sec = secNone;

	size_t pos = 0;
	while (pos <= text.size())
	{
		size_t nl = text.find('\n', pos);
		if (nl == xr_string::npos) nl = text.size();
		xr_string raw = text.substr(pos, nl - pos);
		pos = nl + 1;
		Trim(raw);

		// known sections drop comments; unknown ones are kept verbatim
		xr_string line = raw;
		const size_t sc = FindComment(line);
		if (sc != xr_string::npos) { line = line.substr(0, sc); Trim(line); }

		if (!line.empty() && line[0] == '[')
		{
			const size_t close = line.find(']');
			xr_string sn = (close != xr_string::npos) ? line.substr(1, close - 1) : "";
			Trim(sn);
			for (size_t i = 0; i < sn.size(); ++i)
				if (sn[i] >= 'A' && sn[i] <= 'Z') sn[i] = char(sn[i] - 'A' + 'a');
			if      (sn == "module")		sec = secModule;
			else if (sn == "requires")		sec = secRequires;
			else if (sn == "order")			sec = secOrder;
			else if (sn == "conflicts")		sec = secConflicts;
			else if (sn == "provides_mode")	sec = secProvides;
			else if (sn == "update")		sec = secUpdate;
			else if (sn == "exclude")		sec = secExclude;
			else if (sn == "world")			sec = secWorld;
			else if (sn == "pda")			sec = secPda;
			else { sec = secExtra; m.extra.push_back(raw); }
			continue;
		}
		if (sec == secExtra) { if (!raw.empty()) m.extra.push_back(raw); continue; }
		if (line.empty()) continue;

		if (sec == secPda)
		{
			xr_string key = line, val;
			const size_t eq = line.find('=');
			if (eq != xr_string::npos) { key = line.substr(0, eq); val = line.substr(eq + 1); }
			Trim(key); Trim(val);
			if (key == "layout")	m.pda_layout = val;
			else					m.pda_extra.push_back(line);
			continue;
		}
		if (sec == secModule || sec == secOrder || sec == secProvides || sec == secUpdate)
		{
			xr_string key = line, val;
			const size_t eq = line.find('=');
			if (eq != xr_string::npos) { key = line.substr(0, eq); val = line.substr(eq + 1); }
			Trim(key); Trim(val);
			if (sec == secModule)
			{
				// the three display texts may be quoted; every other value is taken as written
				if      (key == "id")			m.id = val;
				else if (key == "name")			{ if (!Unquote(val, m.name)) m.name = val; }
				else if (key == "version")		m.version = val;
				else if (key == "author")		{ if (!Unquote(val, m.author)) m.author = val; }
				else if (key == "description")	{ if (!Unquote(val, m.description)) m.description = val; }
				else if (key == "website")		m.website = val;
				else if (key == "thumbnail")	m.thumbnail = val;
				else if (key == "tags")			SplitList(m.tags, val.c_str(), ',');
				else if (key == "api")			m.api = val;
				else if (key == "mode")			m.mode = val;
				else if (key == "target")		m.target = val;
				else							m.module_extra.push_back(line);
			}
			else if (sec == secOrder)
			{
				if      (key == "after")	SplitList(m.after, val.c_str(), ',');
				else if (key == "before")	SplitList(m.before, val.c_str(), ',');
			}
			else if (sec == secUpdate)
			{
				if (key == "github")		m.update_github = val;
				else						m.update_extra.push_back(line);
			}
			else
			{
				// each `id` line starts a new declared mode, `title` fills the last one
				if (key == "id")
				{
					SProvidesMode pm;
					pm.id = val;
					m.provides_modes.push_back(pm);
				}
				else if (key == "title" && !m.provides_modes.empty())
					m.provides_modes.back().title = val;
				else
					m.provides_extra.push_back(line);
			}
		}
		else if (sec == secRequires)	m.requires_list.push_back(line);
		else if (sec == secConflicts)	m.conflicts.push_back(line);
		else if (sec == secExclude)		m.exclude.push_back(line);
		else if (sec == secWorld)
		{
			xr_string key = line, val;
			const size_t eq = line.find('=');
			if (eq != xr_string::npos) { key = line.substr(0, eq); val = line.substr(eq + 1); }
			Trim(key); Trim(val);
			SWorld& w = m.world;
			const u32 rule = WorldRuleFromKey(key.c_str());
			if (rule < wrCount)
				// the engine's reading: only an explicit "no" changes anything
				w.keep[rule] = !(val == "false" || val == "0" || val == "no" || val == "off");
			else if (key == "levels")			SplitList(w.levels, val.c_str(), ',');
			else if (key == "area")				SplitList(w.area, val.c_str(), ',');
			else if (key == "locked_levels")	SplitList(w.locked_levels, val.c_str(), ',');
			else if (key == "locked")			SplitList(w.locked, val.c_str(), ',');
			else if (key == "player_choice")
			{
				xr_vector<xr_string> names;
				SplitList(names, val.c_str(), ',');
				for (u32 i = 0; i < names.size(); ++i)
				{
					const u32 r = WorldRuleFromKey(names[i].c_str());
					if (r < wrCount) w.player_choice[r] = true;
				}
			}
			else w.extra.push_back(line);
		}
	}
	return true;
}

LPCSTR EditorMod::WorldRuleKey(u32 rule)
{
	static const char* keys[wrCount] = { "original_npcs", "random_stalkers", "mutants", "npc_quests", "story_quests" };
	return rule < wrCount ? keys[rule] : 0;
}

u32 EditorMod::WorldRuleFromKey(LPCSTR key)
{
	for (u32 i = 0; key && i < wrCount; ++i)
		if (0 == _stricmp(key, WorldRuleKey(i))) return i;
	return wrCount;
}

bool EditorMod::SWorld::Empty() const
{
	for (u32 i = 0; i < wrCount; ++i)
		if (!keep[i] || player_choice[i]) return false;
	return levels.empty() && area.empty() && locked_levels.empty() && locked.empty() && extra.empty();
}

// `width` 11 fits "description", the longest mod.ltx key; the release
// descriptor asks for 13 ("requires_game")
static void AppendKey(xr_string& out, LPCSTR key, LPCSTR val, int width = 11)
{
	char pad[24];
	sprintf_s(pad, "%-*s", width, key);	// keys are short literals only
	out += pad;
	out += " = ";
	out += val;
	out += "\r\n";
}

// a display text key: quoted and escaped when `always`, or when it has to be
static void AppendText(xr_string& out, LPCSTR key, const xr_string& val, bool always)
{
	// CR goes before the choice is made: CRLF text from a paste or a JSON
	// argument becomes one \n, and a stray CR would end the line in the file
	xr_string text;
	for (size_t i = 0; i < val.size(); ++i)
		if (val[i] != '\r') text += val[i];
	AppendKey(out, key, (always || NeedsQuotes(text)) ? Quote(text).c_str() : text.c_str());
}

static void AppendJoined(xr_string& out, LPCSTR key, const xr_vector<xr_string>& v, int width = 11)
{
	xr_string all;
	for (u32 i = 0; i < v.size(); ++i)
	{
		if (i) all += ", ";
		all += v[i];
	}
	AppendKey(out, key, all.c_str(), width);
}

// [world] keys are longer: "random_stalkers"
static const int kWorldKeyWidth = 15;

static u32 s_ManifestGeneration = 0;

bool EditorMod::Save(LPCSTR project_root, const SManifest& m)
{
	if (!project_root || !project_root[0]) return false;
	xr_string t;
	t += "[module]\r\n";
	AppendKey(t, "id", m.id.c_str());
	AppendText(t, "name", m.name, false);
	AppendKey(t, "version", m.version.empty() ? "1.0.0" : m.version.c_str());
	// optional metadata: an empty key is left out. The description is always
	// quoted - it is the one text that routinely holds ';' and line breaks.
	if (!m.author.empty())
		AppendText(t, "author", m.author, false);
	if (!m.description.empty())
		AppendText(t, "description", m.description, true);
	if (!m.website.empty())
		AppendKey(t, "website", m.website.c_str());
	if (!m.thumbnail.empty())
		AppendKey(t, "thumbnail", m.thumbnail.c_str());
	if (!m.tags.empty())
		AppendJoined(t, "tags", m.tags);
	AppendKey(t, "api", m.api.empty() ? "1" : m.api.c_str());
	// empty mode = active in all game modes; the key is omitted then
	if (!m.mode.empty())
		AppendKey(t, "mode", m.mode.c_str());
	// editor-side bookkeeping: the engine ignores it, the export insists on it
	if (!m.target.empty())
		AppendKey(t, "target", m.target.c_str());
	for (u32 i = 0; i < m.module_extra.size(); ++i)
		{ t += m.module_extra[i]; t += "\r\n"; }
	if (!m.update_github.empty() || !m.update_extra.empty())
	{
		t += "\r\n[update]\r\n";
		if (!m.update_github.empty())
			AppendKey(t, "github", m.update_github.c_str());
		for (u32 i = 0; i < m.update_extra.size(); ++i)
			{ t += m.update_extra[i]; t += "\r\n"; }
	}
	if (!m.requires_list.empty())
	{
		t += "\r\n[requires]\r\n";
		for (u32 i = 0; i < m.requires_list.size(); ++i)
			{ t += m.requires_list[i]; t += "\r\n"; }
	}
	if (!m.after.empty() || !m.before.empty())
	{
		t += "\r\n[order]\r\n";
		if (!m.after.empty())	AppendJoined(t, "after", m.after);
		if (!m.before.empty())	AppendJoined(t, "before", m.before);
	}
	if (!m.conflicts.empty())
	{
		t += "\r\n[conflicts]\r\n";
		for (u32 i = 0; i < m.conflicts.size(); ++i)
			{ t += m.conflicts[i]; t += "\r\n"; }
	}
	if (!m.provides_modes.empty() || !m.provides_extra.empty())
	{
		t += "\r\n[provides_mode]\r\n";
		for (u32 i = 0; i < m.provides_modes.size(); ++i)
		{
			AppendKey(t, "id", m.provides_modes[i].id.c_str());
			if (!m.provides_modes[i].title.empty())
				AppendKey(t, "title", m.provides_modes[i].title.c_str());
		}
		for (u32 i = 0; i < m.provides_extra.size(); ++i)
			{ t += m.provides_extra[i]; t += "\r\n"; }
	}
	// editor-side, like `target`: the game ignores the section, the build reads it
	if (!m.exclude.empty())
	{
		t += "\r\n[exclude]\r\n";
		for (u32 i = 0; i < m.exclude.size(); ++i)
			{ t += m.exclude[i]; t += "\r\n"; }
	}
	// the mode's world: every rule spelled out once the section exists, so the file
	// reads as the whole decision, not as a diff against defaults nobody sees
	if (!m.world.Empty())
	{
		const SWorld& w = m.world;
		t += "\r\n[world]\r\n";
		for (u32 i = 0; i < wrCount; ++i)
			AppendKey(t, WorldRuleKey(i), w.keep[i] ? "true" : "false", kWorldKeyWidth);
		xr_vector<xr_string> choices;
		for (u32 i = 0; i < wrCount; ++i)
			if (w.player_choice[i]) choices.push_back(WorldRuleKey(i));
		if (!choices.empty())			AppendJoined(t, "player_choice", choices, kWorldKeyWidth);
		if (!w.levels.empty())			AppendJoined(t, "levels", w.levels, kWorldKeyWidth);
		if (!w.area.empty())			AppendJoined(t, "area", w.area, kWorldKeyWidth);
		if (!w.locked_levels.empty())	AppendJoined(t, "locked_levels", w.locked_levels, kWorldKeyWidth);
		if (!w.locked.empty())			AppendJoined(t, "locked", w.locked, kWorldKeyWidth);
		for (u32 i = 0; i < w.extra.size(); ++i)
			{ t += w.extra[i]; t += "\r\n"; }
	}
	// the module's own PDA interface (EditorPda)
	if (!m.pda_layout.empty() || !m.pda_extra.empty())
	{
		t += "\r\n[pda]\r\n";
		if (!m.pda_layout.empty())
			AppendKey(t, "layout", m.pda_layout.c_str());
		for (u32 i = 0; i < m.pda_extra.size(); ++i)
			{ t += m.pda_extra[i]; t += "\r\n"; }
	}
	if (!m.extra.empty())
	{
		t += "\r\n";
		for (u32 i = 0; i < m.extra.size(); ++i)
			{ t += m.extra[i]; t += "\r\n"; }
	}
	char path[MAX_PATH];
	sprintf_s(path, "%s\\mod.ltx", project_root);
	const bool ok = WriteTextFile(path, t.c_str());
	// whatever read the list before now reads a stale one
	InvalidateExclusions();
	++s_ManifestGeneration;
	return ok;
}

u32 EditorMod::ManifestGeneration()
{
	return s_ManifestGeneration;
}

//------------------------------------------------------------------------------
// what the build leaves out
//------------------------------------------------------------------------------
// "/Docs/Drafts/" and "docs\drafts" are one rule: backslashes, no leading "./"
// or separator, no trailing one
static xr_string NormalizeRel(LPCSTR in)
{
	xr_string r;
	for (LPCSTR p = in ? in : ""; *p; ++p) r += (*p == '/') ? '\\' : *p;
	Trim(r);
	for (;;)
	{
		if (0 == r.compare(0, 2, ".\\"))			r.erase(0, 2);
		else if (!r.empty() && r[0] == '\\')		r.erase(0, 1);
		else break;
	}
	while (!r.empty() && r[r.size() - 1] == '\\')	r.erase(r.size() - 1);
	return r;
}

// '*' and '?' against `text`, case-insensitive; neither crosses `stop` ('\\' in a
// path, 0 for a plain name)
static bool GlobMatch(LPCSTR mask, LPCSTR text, char stop)
{
	LPCSTR star_mask = 0, star_text = 0;
	while (*text)
	{
		if (*mask == '*')	{ star_mask = ++mask; star_text = text; continue; }
		const bool one = (*mask == '?') ? (*text != stop || !stop)
										: (*mask && tolower((u8)*mask) == tolower((u8)*text));
		if (one)			{ ++mask; ++text; continue; }
		// the star takes one more character - never a folder boundary
		if (!star_mask || (stop && *star_text == stop))	return false;
		mask = star_mask;
		text = ++star_text;
	}
	while (*mask == '*') ++mask;
	return !*mask;
}

bool EditorMod::ExcludeMatch(const xr_vector<xr_string>& rules, LPCSTR rel_in, xr_string* rule)
{
	const xr_string rel = NormalizeRel(rel_in);
	if (rel.empty())	return false;
	for (u32 i = 0; i < rules.size(); ++i)
	{
		const xr_string r = NormalizeRel(rules[i].c_str());
		if (r.empty())	continue;
		const bool wild = r.find_first_of("*?") != xr_string::npos;
		const bool path = r.find('\\') != xr_string::npos;
		// the entry itself and every folder above it: a folder takes what is below it
		for (size_t end = rel.find('\\');; end = rel.find('\\', end + 1))
		{
			const xr_string prefix = (end == xr_string::npos) ? rel : rel.substr(0, end);
			bool hit;
			if (!wild)		hit = 0 == _stricmp(prefix.c_str(), r.c_str());
			else if (path)	hit = GlobMatch(r.c_str(), prefix.c_str(), '\\');
			else
			{
				const size_t cut = prefix.rfind('\\');
				hit = GlobMatch(r.c_str(), prefix.c_str() + (cut == xr_string::npos ? 0 : cut + 1), 0);
			}
			if (hit)
			{
				if (rule)	*rule = rules[i];
				return true;
			}
			if (end == xr_string::npos)	break;
		}
	}
	return false;
}

static xr_string				s_ExcludeRoot;
static xr_vector<xr_string>		s_ExcludeRules;
static bool						s_ExcludeValid = false;

const xr_vector<xr_string>& EditorMod::Exclusions(LPCSTR project_root)
{
	// "D:\\Mod" and "D:/mod/" ask for one list
	xr_string root = project_root ? project_root : "";
	for (size_t i = 0; i < root.size(); ++i) if (root[i] == '/') root[i] = '\\';
	while (!root.empty() && root[root.size() - 1] == '\\') root.erase(root.size() - 1);
	if (!s_ExcludeValid || 0 != _stricmp(root.c_str(), s_ExcludeRoot.c_str()))
	{
		SManifest m;
		Load(root.c_str(), m);
		s_ExcludeRules	= m.exclude;
		s_ExcludeRoot	= root;
		s_ExcludeValid	= true;
	}
	return s_ExcludeRules;
}

void EditorMod::InvalidateExclusions()
{
	s_ExcludeValid = false;
	// what the build is made of has changed: the quest index and the character list
	// are both answers to "what is in the build"
	NqDocs::InvalidateProjectIndex();
	EditorCharacters::Invalidate();
}

bool EditorMod::OutsideBuild(LPCSTR project_root, LPCSTR rel_in)
{
	const xr_string rel = NormalizeRel(rel_in);
	if (rel.empty())	return false;
	for (size_t b = 0; b <= rel.size();)
	{
		size_t e = rel.find('\\', b);
		if (e == xr_string::npos) e = rel.size();
		const xr_string seg = rel.substr(b, e - b);
		if (EditorProject::IsToolingEntry(seg.c_str()))	return true;
		// the editor's scratch and the sources are root entries only
		if (b == 0 && (EditorProject::IsEditorOnlyEntry(seg.c_str()) || EditorProject::IsSourceOnlyEntry(seg.c_str())))
			return true;
		b = e + 1;
	}
	return ExcludeMatch(Exclusions(project_root), rel.c_str());
}

EditorMod::EShipState EditorMod::ShipState(LPCSTR project_root, LPCSTR rel_in, xr_string* rule)
{
	const xr_string rel = NormalizeRel(rel_in);
	if (rel.empty())	return shipShipped;
	const xr_string first = rel.substr(0, rel.find('\\'));
	if (EditorProject::IsEditorOnlyEntry(first.c_str()) || EditorProject::IsSourceOnlyEntry(first.c_str()))
		return shipNever;
	xr_string r;
	if (!ExcludeMatch(Exclusions(project_root), rel.c_str(), &r))	return shipShipped;
	if (rule)	*rule = r;
	return 0 == _stricmp(NormalizeRel(r.c_str()).c_str(), rel.c_str()) ? shipExcluded : shipExcludedByRule;
}

bool EditorMod::SetExcluded(LPCSTR project_root, LPCSTR rel_in, bool excluded, xr_string& err)
{
	err.clear();
	SManifest m;
	if (!Load(project_root, m))	{ err = "mod.ltx not found in the project"; return false; }
	const xr_string rel = NormalizeRel(rel_in);
	if (rel.empty())			{ err = "the project root cannot be excluded - that is the whole module"; return false; }
	if (0 == _stricmp(rel.c_str(), "mod.ltx"))
								{ err = "mod.ltx always ships - it is what makes the folder a module"; return false; }
	// stored the way people write paths in a text file
	xr_string stored = rel;
	for (size_t i = 0; i < stored.size(); ++i) if (stored[i] == '\\') stored[i] = '/';

	if (excluded)
	{
		if (ExcludeMatch(m.exclude, rel.c_str()))	return true;	// already out, nothing to write
		// an entry below the new one says nothing any more
		for (u32 i = 0; i < m.exclude.size();)
		{
			const xr_string r = NormalizeRel(m.exclude[i].c_str());
			const bool below = r.size() > rel.size() && r[rel.size()] == '\\' &&
							   0 == _strnicmp(r.c_str(), rel.c_str(), rel.size()) &&
							   r.find_first_of("*?") == xr_string::npos;
			if (below)	m.exclude.erase(m.exclude.begin() + i);
			else		++i;
		}
		m.exclude.push_back(stored);
	}
	else
	{
		for (u32 i = 0; i < m.exclude.size();)
		{
			if (0 == _stricmp(NormalizeRel(m.exclude[i].c_str()).c_str(), rel.c_str()))
				m.exclude.erase(m.exclude.begin() + i);
			else ++i;
		}
		xr_string still;
		if (ExcludeMatch(m.exclude, rel.c_str(), &still))
		{
			err = "still excluded by '" + still + "' in mod.ltx [exclude]";
			return false;
		}
	}
	if (!Save(project_root, m))	{ err = "can't write mod.ltx"; return false; }
	return true;
}

//------------------------------------------------------------------------------
// scaffold
//------------------------------------------------------------------------------
void EditorMod::EnsureScaffold(LPCSTR project_root, LPCSTR project_name)
{
	if (!project_root || !project_root[0]) return;

	// Only the manifest: it is what makes the folder a module at all. NO
	// folders - a module has whatever layout its author gives it ([vfs]
	// publishes any of it), and an empty project stays empty. The folders the
	// engine's fixed channels read (patch\, spawn\, scripts\, levels\,
	// gamedata\) appear when something is put in them, not before.
	char mod[MAX_PATH];
	sprintf_s(mod, "%s\\mod.ltx", project_root);
	if (!FileExists(mod))
	{
		SManifest m;
		char id[128];
		DeriveId(project_name, id, sizeof(id));
		m.id = id;
		m.name = project_name ? project_name : "";
		m.version = "1.0.0";
		m.api = "1";
		if (Save(project_root, m))
			Msg("* [XMS] mod.ltx scaffolded: %s", mod);
	}
}

//------------------------------------------------------------------------------
// export
//------------------------------------------------------------------------------
// ---- the build walk ---------------------------------------------------------
// What a build ships, as ONE list made before anything is written. A clean build
// has to know what it will write to tell what in the target it would throw away,
// and the author is told what the walk passed by: a folder that silently did not
// ship is the next bug report.
struct SBuildFile
{
	xr_string	src;		// absolute
	xr_string	rel;		// project-relative, '\\'
};

struct SBuildWalk
{
	xr_string					root;
	const xr_vector<xr_string>&	exclude;
	xr_vector<SBuildFile>		files;
	xr_vector<xr_string>		left_out;	// project-relative, where the walk stopped

	SBuildWalk(LPCSTR project_root, const xr_vector<xr_string>& rules) : root(project_root), exclude(rules) {}

	bool Skip(const xr_string& rel, const WIN32_FIND_DATAA& fd, bool at_root)
	{
		LPCSTR name = fd.cFileName;
		// the manifest is what makes the folder a module - no rule takes it out
		if (at_root && 0 == _stricmp(name, "mod.ltx"))	return false;
		// the editor's scratch and the sources never ship, and the author knows it
		if (at_root && (EditorProject::IsEditorOnlyEntry(name) || EditorProject::IsSourceOnlyEntry(name)))
			return true;
		// authoring help beside the patches, not content
		if (0 == _stricmp(name, "_readme.txt") && 0 == _strnicmp(rel.c_str(), "patch\\", 6))
			return true;
		// a character's source, like an .object in rawdata: the game reads the files the
		// build generates from it, never the asset itself
		if (!(fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) && NqUtil::Extension(name) == ".npcasset")
			return true;
		// an author key file is a secret: whoever has it publishes updates in the author's name
		if (!(fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) && NqUtil::Extension(name) == ".xkey")
		{
			left_out.push_back(rel + " (a key file never ships)");
			return true;
		}
		// Tooling, a link (it may point anywhere, the project itself included) and
		// what the author excluded: left out, and said so.
		if (EditorProject::IsToolingEntry(name) || (fd.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) ||
			EditorMod::ExcludeMatch(exclude, rel.c_str()))
		{
			left_out.push_back(rel);
			return true;
		}
		// X-Ray cannot open a path longer than this, and neither can the copy
		if (root.size() + rel.size() + 2 >= sizeof(string_path))
		{
			left_out.push_back(rel + " (path too long)");
			return true;
		}
		return false;
	}

	void Walk(const xr_string& rel_dir)
	{
		const xr_string dir = rel_dir.empty() ? root : root + "\\" + rel_dir;
		if (dir.size() + 3 >= sizeof(string_path))	return;
		string_path mask;
		sprintf_s(mask, "%s\\*", dir.c_str());
		WIN32_FIND_DATAA fd;
		HANDLE h = ::FindFirstFileA(mask, &fd);
		if (INVALID_HANDLE_VALUE == h)	return;
		do
		{
			if (!strcmp(fd.cFileName, ".") || !strcmp(fd.cFileName, ".."))	continue;
			const xr_string rel = rel_dir.empty() ? xr_string(fd.cFileName) : rel_dir + "\\" + fd.cFileName;
			if (Skip(rel, fd, rel_dir.empty()))	continue;
			if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)
			{
				Walk(rel);
				continue;
			}
			SBuildFile f;
			f.rel = rel;
			f.src = root + "\\" + rel;
			files.push_back(f);
		} while (::FindNextFileA(h, &fd));
		::FindClose(h);
	}
};

// Writes the walked files below `dst`; `strip` leading characters of each relative
// path are dropped (the flat overlay copies gamedata\\ into its own root)
static int CopyBuildFiles(const SBuildWalk& walk, LPCSTR dst, size_t strip)
{
	int copied = 0;
	xr_string made;		// the last folder created - files come in folder order
	for (u32 i = 0; i < walk.files.size(); ++i)
	{
		const SBuildFile& f = walk.files[i];
		const xr_string rel = f.rel.substr(_min(strip, f.rel.size()));
		const xr_string to = xr_string(dst) + "\\" + rel;
		if (to.size() >= sizeof(string_path))	continue;
		const size_t cut = to.rfind('\\');
		const xr_string folder = to.substr(0, cut);
		if (folder != made)
		{
			CreateDirChain(folder.c_str());
			made = folder;
		}
		if (::CopyFileA(f.src.c_str(), to.c_str(), FALSE))	++copied;
		else Msg("! [XMS] build: cannot copy %s (error %u)", f.rel.c_str(), ::GetLastError());
	}
	return copied;
}

// Where a folder really is: links, junctions and 8.3 names resolved, so two spellings
// of one place compare equal. False when it cannot be opened.
static bool RealFolder(LPCSTR path, xr_string& out)
{
	HANDLE h = ::CreateFileA(path, FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
		NULL, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, NULL);
	if (INVALID_HANDLE_VALUE == h)	return false;
	char buf[2048];
	const DWORD n = ::GetFinalPathNameByHandleA(h, buf, sizeof(buf), FILE_NAME_NORMALIZED | VOLUME_NAME_DOS);
	::CloseHandle(h);
	if (!n || n >= sizeof(buf))	return false;
	out = buf;
	if (0 == out.compare(0, 4, "\\\\?\\"))	out.erase(0, 4);
	while (!out.empty() && out[out.size() - 1] == '\\')	out.erase(out.size() - 1);
	return true;
}

// Where `path` would really be once created: the deepest part of it that exists,
// resolved, with the rest appended as written - so a refused build creates nothing
static bool ProjectedRealPath(LPCSTR path, xr_string& out)
{
	xr_string head = path ? path : "", tail;
	while (!head.empty())
	{
		if (DirExists(head.c_str()) && RealFolder(head.c_str(), out))
		{
			if (!tail.empty()) { out += "\\"; out += tail; }
			return true;
		}
		const size_t cut = head.rfind('\\');
		if (cut == xr_string::npos)	return false;
		tail = tail.empty() ? head.substr(cut + 1) : head.substr(cut + 1) + "\\" + tail;
		head.erase(cut);
	}
	return false;
}

static bool SameOrBelow(const xr_string& path, const xr_string& folder)
{
	return path.size() >= folder.size() && 0 == _strnicmp(path.c_str(), folder.c_str(), folder.size()) &&
		   (path.size() == folder.size() || path[folder.size()] == '\\');
}

static int CountFilesRec(const char* dir, const char* skip_file)
{
	int files = 0;
	string_path mask;
	sprintf_s(mask, "%s\\*", dir);
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (h == INVALID_HANDLE_VALUE) return 0;
	do
	{
		if (!strcmp(fd.cFileName, ".") || !strcmp(fd.cFileName, "..")) continue;
		string_path a;
		sprintf_s(a, "%s\\%s", dir, fd.cFileName);
		if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)
			files += CountFilesRec(a, skip_file);
		else if (!skip_file || 0 != _stricmp(fd.cFileName, skip_file))
			++files;
	} while (::FindNextFileA(h, &fd));
	::FindClose(h);
	return files;
}

// The one place a mod must NEVER land: inside the game's own gamedata. The
// whole point of the module system is that a mod lives in its own folder and
// nothing ever merges into the game tree - so a target that would put files
// there is refused, not warned about.
static bool InsideGameGamedata(LPCSTR path)
{
	LPCSTR game = EditorProject::GameRoot();
	if (!game || !game[0]) return false;
	char guard[MAX_PATH];
	sprintf_s(guard, "%s\\gamedata", game);
	const size_t n = xr_strlen(guard);
	return 0 == _strnicmp(path, guard, n) && (path[n] == 0 || path[n] == '\\');
}

// A module that brings its OWN game mode has to put the checkbox on the
// new-game screen itself - the engine cannot, that screen is built in Lua by
// the game. Generated files do it, one set per module however many modes it
// declares:
//
//   gamedata\configs\ui\<module id>_modes<aspect>.xml   the module's own layout
//                                   with the checkboxes and their captions, one
//                                   per aspect-ratio variant of the screen
//   gamedata\configs\text\rus\<module id>_modes.xml   the captions
//   scripts\mode_register.script    creates the controls at runtime and writes
//                                   the [character_creation] keys the engine
//                                   turns back into active XMS modes
//
// The names carry the module, not the mode: renaming a mode rewrites the same
// files instead of leaving a stale checkbox behind in the project. The layouts
// and the string table land in the game's shared namespace, which is why they
// carry the module id - two modules must not fight over one file. The legacy
// patch\xms_modes.xmlp is only cleaned up.
//
// mode_register is the name the engine reserves for generated startup code;
// scripts\register.script stays the author's own and is never touched here.
//
// Written only when the manifest declares a mode; a module targeting an
// existing mode needs none of it.
void EditorMod::GetOwnedFiles(LPCSTR project_root, xr_vector<xr_string>& rel)
{
	rel.push_back("mod.ltx");
	rel.push_back("patch\\xms_modes.xmlp");	// legacy generator output, still cleaned
	rel.push_back("scripts\\mode_register.script");
	// these carry the module id (they land in the game's shared namespace) -
	// resolve it from the manifest on disk
	SManifest m;
	if (project_root && Load(project_root, m) && !m.id.empty())
	{
		rel.push_back(xr_string("gamedata\\configs\\text\\rus\\") + m.id + "_modes.xml");
		rel.push_back(xr_string("gamedata\\configs\\ui\\") + m.id + "_modes.xml");
		rel.push_back(xr_string("gamedata\\configs\\ui\\") + m.id + "_modes_16.xml");
		// the generated half of every character: hidden from the browser and
		// removed with the last .npcasset, exactly like the mode files above
		EditorCharacters::GeneratedFiles(m.id.c_str(), rel);
	}
}

static bool WriteModeRegistration(LPCSTR project_root, const EditorMod::SManifest& m, xr_string& err)
{
	// A declaration that is GONE has to take its generated files with it:
	// they are hidden from the browser, so nothing else would ever remove
	// them, and a stale invisible xmlp still puts a checkbox in the game.
	if (m.provides_modes.empty())
	{
		string_path stale;
		sprintf_s(stale, "%s\\patch\\xms_modes.xmlp", project_root);
		if (::DeleteFileA(stale)) Msg("* [XMS] mode declaration gone - removed %s", stale);
		sprintf_s(stale, "%s\\scripts\\mode_register.script", project_root);
		if (::DeleteFileA(stale)) Msg("* [XMS] mode declaration gone - removed %s", stale);
		if (!m.id.empty())
		{
			sprintf_s(stale, "%s\\gamedata\\configs\\text\\rus\\%s_modes.xml", project_root, m.id.c_str());
			if (::DeleteFileA(stale)) Msg("* [XMS] mode declaration gone - removed %s", stale);
			sprintf_s(stale, "%s\\gamedata\\configs\\ui\\%s_modes.xml", project_root, m.id.c_str());
			if (::DeleteFileA(stale)) Msg("* [XMS] mode declaration gone - removed %s", stale);
			sprintf_s(stale, "%s\\gamedata\\configs\\ui\\%s_modes_16.xml", project_root, m.id.c_str());
			if (::DeleteFileA(stale)) Msg("* [XMS] mode declaration gone - removed %s", stale);
		}
		return true;
	}

	// Never trust the manifest here: it may have been hand-written or made by
	// an older build, and these ids become file names, XML node names and Lua
	// identifiers. An id like "..\..\x" or one with a quote in it would write
	// outside the project or emit a file the game cannot parse.
	xr_vector<EditorMod::SProvidesMode> modes;
	for (u32 i = 0; i < m.provides_modes.size(); ++i)
	{
		const xr_string& id = m.provides_modes[i].id;
		if (id.empty()) continue;
		if (!EditorMod::ValidateModeId(id.c_str()) || id.size() > 64)
		{
			err  = "invalid declared mode id '" + id + "' (allowed: a-z 0-9 _ . -, up to 64 chars)";
			return false;
		}
		modes.push_back(m.provides_modes[i]);
	}
	if (modes.empty()) return true;

	string_path path;
	xr_string body;
	body.resize(8192);

	// ---- xml: the module's OWN layout files with the fallback checkboxes ----
	// The primary path needs no xml at all (the Refined dropdown builds its
	// row in code); these files exist for the checkbox FALLBACK on installs
	// without that layer. They are the module's own files - the screen's xml
	// is never patched, so nothing here can fight the game's (or Refined's)
	// idea of where the frame sits. Coordinates come from measuring the
	// layout the linked game really ships, one file per aspect variant; the
	// engine picks the _16 one on wide screens automatically.
	static const char* kLayouts[]  = { "ui_mm_faction_select_16.xml", "ui_mm_faction_select.xml" };
	static const char* kSuffixes[] = { "_16", "" };
	int targets = 0;
	for (int t = 0; t < 2; ++t)
	{
		string_path rel;
		sprintf_s(rel, "configs\\ui\\%s", kLayouts[t]);
		EditorGameModes::SSlot slot;
		xr_string slot_err;
		if (!EditorGameModes::SuggestSlot(rel, (int)modes.size(), m.id.c_str(), slot, slot_err))
			continue;
		++targets;

		xr_string ui = "<?xml version=\"1.0\" encoding=\"windows-1251\"?>\r\n<w>\r\n";
		for (u32 i = 0; i < modes.size(); ++i)
		{
			const xr_string node	= "check_" + modes[i].id + "_mode";
			const xr_string string_id = "st_cap_" + node;
			const int dy = int(i) * slot.step;
			sprintf_s(&body[0], body.size(),
				"\t<cap_%s x=\"%d\" y=\"%d\" width=\"%d\" height=\"%d\" stretch=\"1\">\r\n"
				"\t\t<text r=\"170\" g=\"170\" b=\"170\" font=\"letterica16\" align=\"l\" vert_align=\"c\">%s</text>\r\n"
				"\t</cap_%s>\r\n"
				"\t<%s x=\"%d\" y=\"%d\" width=\"%d\" height=\"%d\" stretch=\"1\">\r\n"
				"\t</%s>\r\n",
				node.c_str(), slot.cap_x, slot.cap_y + dy, slot.cap_w, slot.cap_h,
				string_id.c_str(), node.c_str(),
				node.c_str(), slot.check_x, slot.check_y + dy, slot.check_w, slot.check_h,
				node.c_str());
			ui += body.c_str();
		}
		ui += "</w>\r\n";

		sprintf_s(path, "%s\\gamedata\\configs\\ui", project_root);
		CreateDirChain(path);
		sprintf_s(path, "%s\\gamedata\\configs\\ui\\%s_modes%s.xml", project_root, m.id.c_str(), kSuffixes[t]);
		if (!WriteTextFile(path, ui.c_str())) { err = "cannot write the mode layout xml"; return false; }
	}
	if (!targets)
	{
		err = "the linked game has no new-game screen layout - nowhere to put the mode checkbox";
		return false;
	}

	// the old generator emitted a patch against the screen's xml; a stale one
	// would still move the frame around, so it dies here
	{
		string_path stale;
		sprintf_s(stale, "%s\\patch\\xms_modes.xmlp", project_root);
		if (::DeleteFileA(stale)) Msg("* [XMS] legacy xms_modes.xmlp removed - the fallback uses the module's own layout now");
	}

	// ---- text: what the player reads ----------------------------------------
	// String tables are cp1251 like the rest of the game; the manifest caption
	// comes out of ImGui as UTF-8, so it is converted on the way out.
	xr_string strings = "<?xml version=\"1.0\" encoding=\"windows-1251\"?>\r\n<string_table>\r\n";
	for (u32 i = 0; i < modes.size(); ++i)
	{
		const xr_string& title	= modes[i].title.empty() ? modes[i].id : modes[i].title;
		// the conversion below is lossy by design (best-fit, then '?'); an export
		// driven from MCP never sees the field's inline warning, so say it here
		xr_string outside;
		if (!NqUtil::Cp1251Covers(title, outside))
			Msg("! [XMS] mode '%s': the caption has characters outside cp1251 (%s) - the player will "
				"see a look-alike or '?'", modes[i].id.c_str(), outside.c_str());
		const xr_string caption	= XmlEscape(Utf8ToCp1251(title.c_str()).c_str());
		sprintf_s(&body[0], body.size(),
			"\t<string id=\"st_cap_check_%s_mode\">\r\n"
			"\t\t<text>%s</text>\r\n"
			"\t</string>\r\n",
			modes[i].id.c_str(), caption.c_str());
		strings += body.c_str();
	}
	strings += "</string_table>\r\n";
	sprintf_s(path, "%s\\gamedata\\configs\\text\\rus", project_root);
	CreateDirChain(path);
	sprintf_s(path, "%s\\gamedata\\configs\\text\\rus\\%s_modes.xml", project_root, m.id.c_str());
	if (!WriteTextFile(path, strings.c_str())) { err = "cannot write the mode caption"; return false; }

	// ---- script: create the control and store the choice --------------------
	// Runs from the engine's registrar pass (xms.load_registrars). faction_ui is
	// a luabind class, so its methods are wrapped by assignment - xms.hook only
	// resolves plain _G tables. Everything is pcall-guarded: a game whose screen
	// was replaced by another mod must still reach the menu, just without the
	// checkbox, and the log says so.
	//
	// Two paths, tried in order:
	//   1. Refined's module-mode dropdown (dead_air_x64_mode_select) - modes
	//      go into ONE shared combo on the new-game screen; that script also
	//      writes the keys on start.
	//   2. plain DA without that layer - the module's own checkbox, appended
	//      to the modes column (the xmlp+strings this export writes anyway).
	sprintf_s(&body[0], body.size(),
		"-- generated by XFined Editor for module '%s'\r\n"
		"-- Puts this module's game mode(s) on the new-game screen.\r\n"
		"-- Rewritten on every export - edit the manifest, not this file.\r\n"
		"-- (scripts\\register.script is yours; nothing here touches it.)\r\n"
		"local MODES = {\r\n",
		m.id.c_str());
	xr_string lua = body.c_str();
	for (u32 i = 0; i < modes.size(); ++i)
	{
		// reg stays "main_dialog:check_<id>_mode" - it is the DISCOVERY
		// contract other systems (the exclusivity group) build from the mode
		// id; xml_* are the node names inside the module's own layout file
		sprintf_s(&body[0], body.size(),
			"    { id = \"%s\", reg = \"main_dialog:check_%s_mode\","
			" xml_check = \"check_%s_mode\", xml_cap = \"cap_check_%s_mode\","
			" key = \"new_game_%s_mode\" },\r\n",
			modes[i].id.c_str(), modes[i].id.c_str(), modes[i].id.c_str(),
			modes[i].id.c_str(), modes[i].id.c_str());
		lua += body.c_str();
	}
	lua +=
		"}\r\n"
		"\r\n"
		"-- Refined's screen keeps ONE dropdown for module modes - join it when the\r\n"
		"-- layer is present (it also writes the keys on start). Otherwise fall\r\n"
		"-- back to this module's own checkbox, so the mod still works on a plain\r\n"
		"-- DA install.\r\n"
		"local ok_dd, dd = pcall(function() return dead_air_x64_mode_select end)\r\n"
		"if ok_dd and dd and dd.add_mode then\r\n"
		"    for i = 1, #MODES do\r\n"
		"        dd.add_mode(MODES[i].id, \"st_cap_check_\" .. MODES[i].id .. \"_mode\")\r\n"
		"    end\r\n"
		"    xms.log(\"game mode(s) joined the module dropdown: \" .. #MODES)\r\n"
		"    return\r\n"
		"end\r\n"
		"\r\n"
		"local ok_screen, screen = pcall(function() return ui_mm_faction_select end)\r\n"
		"local cls = ok_screen and screen and screen.faction_ui\r\n"
		"if not cls then\r\n"
		"    xms.log(\"! xms modes: no new-game screen, they can only be set from the console\")\r\n"
		"    return\r\n"
		"end\r\n"
		"\r\n"
		"-- The fallback controls come from the MODULE'S OWN layout file - the\r\n"
		"-- screen's xml is never patched, so nothing fights over its frame. The\r\n"
		"-- engine picks the _16 variant on wide screens by itself.\r\n"
		"local base_init = cls.InitControls\r\n"
		"if type(base_init) == \"function\" then\r\n"
		"    cls.InitControls = function(self, f)\r\n"
		"        base_init(self, f)\r\n"
		"        self.xms_checks = {}\r\n"
		"        local xml = CScriptXmlInit()\r\n"
		"        xml:ParseFile(\"%MODID%_modes.xml\")\r\n"
		"        for i = 1, #MODES do\r\n"
		"            local m = MODES[i]\r\n"
		"            local ok, err = pcall(function()\r\n"
		"                xml:InitStatic(m.xml_cap, self.dialog)\r\n"
		"                self.xms_checks[m.id] = xml:InitCheck(m.xml_check, self.dialog)\r\n"
		"                self:Register(self.xms_checks[m.id], m.reg)\r\n"
		"            end)\r\n"
		"            if not ok then xms.log(\"! \" .. m.id .. \" checkbox: \" .. tostring(err)) end\r\n"
		"        end\r\n"
		"    end\r\n"
		"else\r\n"
		"    xms.log(\"! xms modes: InitControls missing, checkboxes skipped\")\r\n"
		"end\r\n"
		"\r\n"
		"-- The choice, written where the engine reads active modes from. Only the\r\n"
		"-- literal true counts as ticked, on this side and in the game's own\r\n"
		"-- is_*_mode helpers alike.\r\n"
		"local base_start = cls.OnStartGame\r\n"
		"if type(base_start) == \"function\" then\r\n"
		"    cls.OnStartGame = function(self, ...)\r\n"
		"        pcall(function()\r\n"
		"            local cfg = axr_main and axr_main.config\r\n"
		"            if not cfg then return end\r\n"
		"            for i = 1, #MODES do\r\n"
		"                local m = MODES[i]\r\n"
		"                local ck = self.xms_checks and self.xms_checks[m.id]\r\n"
		"                cfg:w_value(\"character_creation\", m.key, ck and ck:GetCheck() and true or nil)\r\n"
		"            end\r\n"
		"        end)\r\n"
		"        return base_start(self, ...)\r\n"
		"    end\r\n"
		"else\r\n"
		"    xms.log(\"! xms modes: OnStartGame missing, nothing will be stored\")\r\n"
		"end\r\n"
		"\r\n"
		"xms.log(\"game mode(s) registered on the new-game screen: \" .. #MODES)\r\n";

	// the fallback layout file carries the module id in its name
	for (size_t at = lua.find("%MODID%"); at != xr_string::npos; at = lua.find("%MODID%"))
		lua.replace(at, 7, m.id);

	sprintf_s(path, "%s\\scripts", project_root);
	CreateDirChain(path);
	sprintf_s(path, "%s\\scripts\\mode_register.script", project_root);
	if (!WriteTextFile(path, lua.c_str())) { err = "cannot write the mode registrar script"; return false; }

	Msg("* [XMS] %d game mode(s) registered for %d layout(s)", int(modes.size()), targets);
	return true;
}

// Empties a folder, keeping the folder itself. The module export starts from
// this: a build is CLEAN, so whatever the previous one left - a part the author
// has since deleted, a file they renamed, a whole folder that is no longer part
// of the project - is gone before the new one is written. Mirroring folder by
// folder could not do that: it only ever visited the parts the project still
// had, so anything the project stopped having stayed in the module forever.
static int WipeTree(LPCSTR dir)
{
	int removed = 0;
	WIN32_FIND_DATAA fd;
	string_path mask;
	sprintf_s(mask, "%s\\*", dir);
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (INVALID_HANDLE_VALUE == h) return 0;
	do
	{
		if (!strcmp(fd.cFileName, ".") || !strcmp(fd.cFileName, "..")) continue;
		string_path p;
		sprintf_s(p, "%s\\%s", dir, fd.cFileName);
		if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)
		{
			// a link goes, what it points at stays - that may be anything, the
			// author's working copy included
			if (!(fd.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT))
				removed += WipeTree(p);
			::RemoveDirectoryA(p);
		}
		else
		{
			// a read-only file (came off a CD, or the game marked it) still goes
			::SetFileAttributesA(p, FILE_ATTRIBUTE_NORMAL);
			if (::DeleteFileA(p)) ++removed;
		}
	} while (::FindNextFileA(h, &fd));
	::FindClose(h);
	return removed;
}

// How much of the target the clean build would throw away: every file there that
// this build does not write, root level included, and every link as one - asked
// before a folder that may not be the editor's own is wiped. `names` gets the first
// ten of them, for the question that names what goes.
static int CountStaleRec(const xr_string& dir, const xr_string& rel_dir, const xr_set<xr_string>& will,
						 xr_vector<xr_string>& names)
{
	int n = 0;
	if (dir.size() + 3 >= sizeof(string_path))	return 0;
	string_path mask;
	sprintf_s(mask, "%s\\*", dir.c_str());
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (INVALID_HANDLE_VALUE == h)	return 0;
	auto stale = [&](const xr_string& rel)
	{
		++n;
		if (names.size() < 10) names.push_back(rel);
	};
	do
	{
		if (!strcmp(fd.cFileName, ".") || !strcmp(fd.cFileName, ".."))	continue;
		const xr_string rel = rel_dir.empty() ? xr_string(fd.cFileName) : rel_dir + "\\" + fd.cFileName;
		if (fd.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT)	{ stale(rel); continue; }
		if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)
		{
			n += CountStaleRec(dir + "\\" + fd.cFileName, rel, will, names);
			continue;
		}
		xr_string key = rel;
		xr_strlwr(&key[0]);
		if (!will.count(key))	stale(rel);
	} while (::FindNextFileA(h, &fd));
	::FindClose(h);
	return n;
}

// what the last clean build refused to delete, for the UI's second question
static xr_vector<xr_string>	s_StaleNames;
static int					s_StaleCount	= 0;


bool EditorMod::Export(LPCSTR project_root, LPCSTR target_root, bool flat,
					   int& files, xr_string& out_path, xr_string& err, bool confirmed,
					   xr_string* warnings, xr_vector<xr_string>* left_out)
{
	files = 0; out_path.clear(); err.clear();
	if (warnings) warnings->clear();
	if (left_out) left_out->clear();
	if (!project_root || !project_root[0])	{ err = "no active project"; return false; }

	SManifest m;
	if (!Load(project_root, m))				{ err = "mod.ltx not found in the project"; return false; }
	if (!ValidateId(m.id.c_str()))			{ err = "module id missing or invalid in mod.ltx (allowed: a-z 0-9 _ . -)"; return false; }

	// A finished module always states which game it is for. "Nothing chosen"
	// is not a third option - it is how content ends up applied in a campaign
	// it was never built for.
	if (m.mode.empty() && m.provides_modes.empty() && 0 != _stricmp(m.target.c_str(), "default"))
	{
		err = "no target game mode: open Mod > Mod Settings, Game Mode, and pick one "
			  "(the ordinary game, a mode of the linked game, or a new mode this module adds)";
		return false;
	}

	// Quest and behaviour graphs ship as they are, so a broken one would only be
	// found by the player: every *.nqasset and *.behasset must validate before
	// anything is copied.
	{
		xr_vector<xr_string> nq_log;
		int nq_errors = 0, nq_warnings = 0;
		const bool nq_ok = NqExport::ValidateProject(project_root, err, nq_log, &nq_errors, &nq_warnings);
		if (warnings && nq_warnings)
		{
			// the lines the author needs to read, not a count they have to go looking for
			*warnings = NqUtil::Format("quest warnings: %d", nq_warnings);
			for (u32 i = 0; i < nq_log.size(); ++i)
			{
				if (nq_log[i].find(": E") != xr_string::npos) continue;
				*warnings += "\n  ";
				*warnings += nq_log[i];
			}
		}
		if (!nq_ok) return false;
	}

	// The mode's world: a second Player Start or a fourth new-game choice is a
	// decision the game would make for the author (the first start wins, the fourth
	// row never shows) - refused; a level or transition the game lacks is a warning.
	{
		xr_vector<xr_string> problems;
		EditorWorld::Validate(project_root, m, problems);
		xr_string errors;
		for (const xr_string& p : problems)
		{
			if (p[0] == 'E')
			{
				if (!errors.empty()) errors += "\n";
				errors += p;
			}
			else if (warnings)
			{
				if (!warnings->empty()) *warnings += "\n";
				*warnings += p;
			}
		}
		if (!errors.empty())
		{
			err = errors;
			return false;
		}
	}

	// The module's own PDA interface: the game refuses a layout with an error and keeps
	// its stock PDA, so the build refuses it first, where the author can still fix it.
	{
		xr_vector<xr_string> pda_errors, pda_warnings;
		EditorPda::ValidateForBuild(project_root, pda_errors, pda_warnings);
		for (const xr_string& p : pda_warnings)
			if (warnings)
			{
				if (!warnings->empty()) *warnings += "\n";
				*warnings += p;
			}
		if (!pda_errors.empty())
		{
			err = "The PDA layout has errors:";
			for (const xr_string& p : pda_errors) { err += "\n"; err += p; }
			return false;
		}
	}

	// A declared mode is worthless until the player can tick it, so the files
	// that put it on the new-game screen are generated as part of the export.
	if (!flat && !WriteModeRegistration(project_root, m, err))
		return false;

	// The module's own characters: one .npcasset each becomes a character_desc
	// entry, a profile, a string table, a spawn section and the .ltxp patch
	// that makes the game read all of it. Generated here so that a build always
	// ships what the assets currently say - and a deleted character takes its
	// generated half with it.
	{
		int characters = 0;
		if (!EditorCharacters::Generate(project_root, m.id.c_str(), characters, err))
			return false;
	}

	char target[MAX_PATH] = {};
	NormalizePath(target_root ? target_root : "", target, sizeof(target));
	if (!target[0])							{ err = "target folder not set"; return false; }
	if (xr_strlen(target) + m.id.size() + 32 >= MAX_PATH)
											{ err = "target path too long"; return false; }

	// A module goes to <game>\modules\, NOT to mods\: that one is the folder
	// JSGME manages, and a module sitting there is a module JSGME offers to
	// copy over the game - the merge a module exists to avoid. A flat overlay
	// is the opposite case: that IS a JSGME mod, so it keeps its own name.
	string_path dst;
	if (flat)	sprintf_s(dst, "%s\\gamedata_%s", target, m.id.c_str());
	else		sprintf_s(dst, "%s\\modules\\%s", target, m.id.c_str());
	if (InsideGameGamedata(dst) || InsideGameGamedata(target))
	{
		err = "target is inside the game's gamedata - a mod must stay in its own folder "
			  "(export to the game ROOT: the module goes to <game>\\modules\\<id>)";
		return false;
	}
	// The clean build below empties the target first. A target that is a link - one
	// made so the game runs the working copy directly - or that is, holds or sits in
	// the project would take the author's sources with it. Judged on where the target
	// would be, before a single folder of it is created.
	{
		xr_string real_dst, real_src;
		const bool known = ProjectedRealPath(dst, real_dst) && RealFolder(project_root, real_src);
		const DWORD attr = ::GetFileAttributesA(dst);
		if (INVALID_FILE_ATTRIBUTES != attr && (attr & FILE_ATTRIBUTE_REPARSE_POINT))
		{
			err = "the target folder is a link";
			if (known) { err += " to "; err += real_dst; }
			err += " - a build empties it first, and would empty what it points at. Remove the link, or build to another folder";
			out_path = dst;
			return false;
		}
		// inside the editor's own scratch is fine - the release stage lives in temp\,
		// and no build walks it
		bool scratch = false;
		if (known && real_dst.size() > real_src.size() && SameOrBelow(real_dst, real_src))
		{
			xr_string first = real_dst.substr(real_src.size() + 1);
			const size_t cut = first.find('\\');
			if (cut != xr_string::npos) first.erase(cut);
			scratch = EditorProject::IsEditorOnlyEntry(first.c_str());
		}
		if (known && ((SameOrBelow(real_dst, real_src) && !scratch) || SameOrBelow(real_src, real_dst)))
		{
			err = "the target folder and the project overlap (" + real_dst + ") - the build would write into its own sources, or wipe them";
			out_path = dst;
			return false;
		}
	}
	CreateDirChain(dst);
	if (!DirExists(dst))					{ err = "can't create target folder"; return false; }

	SBuildWalk walk(project_root, m.exclude);
	if (flat)
	{
		// the overlay is gamedata alone, filtered exactly like a module build
		if (ExcludeMatch(m.exclude, "gamedata"))
			walk.left_out.push_back("gamedata");
		else
			walk.Walk("gamedata");
		files += CopyBuildFiles(walk, dst, 9);	// "gamedata\\" off the front

		// XMS-only content is not representable as a plain gamedata overlay
		string_path d_patch, d_spawn, d_scripts, d_levels;
		sprintf_s(d_patch, "%s\\patch", project_root);
		sprintf_s(d_spawn, "%s\\spawn", project_root);
		sprintf_s(d_scripts, "%s\\scripts", project_root);
		sprintf_s(d_levels, "%s\\levels", project_root);
		const int c_patch	= CountFilesRec(d_patch, "_readme.txt");
		const int c_spawn	= CountFilesRec(d_spawn, 0);
		const int c_scripts	= CountFilesRec(d_scripts, 0);
		const int c_levels	= CountFilesRec(d_levels, 0);
		// quests are module-only content wherever they sit: the NQ runtime finds
		// them through xms.list_files(<module id>), i.e. under <game>\modules\<id>.
		// A flat overlay is not a module and is never scanned, so a copy that
		// lands in gamedata\ is inert and one outside gamedata\ is not copied.
		xr_vector<NqDocs::SProjectQuest> nq_quests;
		NqDocs::ProjectQuests(nq_quests, true);
		const int c_quests	= (int)nq_quests.size();

		xr_string rep = "XMS flat export report - module '";
		rep += m.id;
		rep += "'\r\n\r\n";
		char line[128];
		if (!c_patch && !c_spawn && !c_scripts && !c_levels && !c_quests && m.mode.empty() && m.provides_modes.empty())
			rep += "fully vanilla-compatible\r\n";
		else
		{
			// a flat overlay has no way to add a checkbox to the new-game screen
			if (!m.provides_modes.empty())
			{
				rep += "declared mode: ";
				rep += m.provides_modes[0].id;
				rep += " - NOT registered: a flat overlay cannot add it to the "
					   "new-game screen, export as a module for that\r\n";
			}
			if (c_patch)	{ sprintf_s(line, "patch: %d file(s) - NOT exported (XMS-only)\r\n", c_patch); rep += line; }
			if (c_spawn)	{ sprintf_s(line, "spawn: %d file(s) - NOT exported (XMS-only)\r\n", c_spawn); rep += line; }
			if (c_scripts)	{ sprintf_s(line, "scripts: %d file(s) - NOT exported (XMS-only)\r\n", c_scripts); rep += line; }
			if (c_levels)	{ sprintf_s(line, "levels: %d overlay file(s) - NOT exported (XMS-only)\r\n", c_levels); rep += line; }
			if (c_quests)	{ sprintf_s(line, "graphs: %d .nqasset/.behasset - NOT usable flat (the NQ runtime only scans <game>\\modules\\<id>)\r\n", c_quests); rep += line; }
			if (!m.mode.empty())
			{
				rep += "target mode: ";
				rep += m.mode;
				rep += " - engine gating is XMS-only (NOT exported flat)\r\n";
			}
		}
		string_path rp;
		sprintf_s(rp, "%s\\_xms_export_report.txt", dst);
		WriteTextFile(rp, rep.c_str());
	}
	else
	{
		// What ships is "everything the author made", not a fixed list of five
		// folder names: a module may keep any layout it likes and publish it
		// through mod.ltx [vfs]. What stays behind: the editor's own scratch, the
		// sources the game cannot read (rawdata scenes, .object files), tooling
		// (.git and its kind) and whatever the author put in [exclude].
		walk.Walk(xr_string());

		// A file the build itself generates is part of how the module works: a rule
		// that takes one out would ship a module that silently does not.
		{
			xr_vector<xr_string> owned;
			GetOwnedFiles(project_root, owned);
			for (u32 i = 0; i < owned.size(); ++i)
			{
				xr_string rule;
				string_path abs;
				sprintf_s(abs, "%s\\%s", project_root, owned[i].c_str());
				if (FileExists(abs) && ExcludeMatch(m.exclude, owned[i].c_str(), &rule))
				{
					err = "mod.ltx [exclude] '" + rule + "' takes out " + owned[i] +
						  ", which the build generates for the module to work - exclude something narrower";
					return false;
				}
			}
		}

		// The build is CLEAN: the target module folder is emptied first, so what
		// ends up there is exactly this build and nothing a previous one left.
		// That is destructive when the target is somebody else's module that
		// happens to share the id - so say so, and only wipe once told to.
		if (!confirmed && DirExists(dst))
		{
			xr_set<xr_string> will;
			for (u32 i = 0; i < walk.files.size(); ++i)
			{
				xr_string key = walk.files[i].rel;
				xr_strlwr(&key[0]);
				will.insert(key);
			}
			xr_vector<xr_string> names;
			if (const int stale = CountStaleRec(dst, xr_string(), will, names))
			{
				s_StaleNames = names;
				s_StaleCount = stale;
				err = "target already holds a module: ";
				char n[64]; sprintf_s(n, "%d file(s) there are not part of this build", stale);
				err += n;
				out_path = dst;
				return false;	// caller re-runs with `confirmed` once the user agrees
			}
		}

		if (const int wiped = WipeTree(dst))
			Msg("* [XMS] clean build: %d stale file(s) removed from %s", wiped, dst);

		files += CopyBuildFiles(walk, dst, 0);
	}

	if (!walk.left_out.empty())
	{
		xr_string list;
		for (u32 i = 0; i < walk.left_out.size() && i < 12; ++i)
		{
			if (i) list += ", ";
			list += walk.left_out[i];
		}
		if (walk.left_out.size() > 12) list += NqUtil::Format(" and %u more", u32(walk.left_out.size() - 12));
		Msg("* [XMS] left out of the build: %s", list.c_str());
	}
	if (left_out) *left_out = walk.left_out;

	// The editor takes any name (its file APIs speak UTF-8); the game reads a module
	// through the ANSI APIs of a cp1251 system, where a character cp1251 lacks comes
	// back as '?' and the file never opens. It still ships - it is the author's.
	{
		xr_vector<xr_string> unreadable;
		for (u32 i = 0; i < walk.files.size(); ++i)
		{
			xr_string bad;
			if (!NqUtil::Cp1251Covers(walk.files[i].rel, bad))
				unreadable.push_back(walk.files[i].rel + " (" + bad + ")");
		}
		for (u32 i = 0; i < unreadable.size(); ++i)
			Msg("! [XMS] build: the game cannot open %s - a character outside cp1251", unreadable[i].c_str());
		if (warnings && !unreadable.empty())
		{
			if (!warnings->empty()) *warnings += "\n";
			*warnings += NqUtil::Format("file names the game cannot open (characters outside cp1251): %u",
										u32(unreadable.size()));
			for (u32 i = 0; i < unreadable.size() && i < 12; ++i)
			{
				*warnings += "\n  ";
				*warnings += unreadable[i];
			}
			if (unreadable.size() > 12) *warnings += NqUtil::Format("\n  and %u more", u32(unreadable.size() - 12));
		}
	}

	out_path = dst;
	Msg("* [XMS] export %s: %d file(s) -> %s", flat ? "flat" : "module", files, dst);
	return true;
}

//------------------------------------------------------------------------------
// release packaging
//
// The descriptor and index names and formats, the package names and the package
// layout are a FROZEN contract with the game's module updater - game repo,
// docs/dead-air/MOD_UPDATES.md, section 2. It changes by adding optional keys
// and records and in no other way: a published release has to install forever.
//
// The game never reads a package as a zip. It asks GitHub for byte ranges the
// file index names, so the index states where the data REALLY sits: offsets are
// read back from the finished archive, never worked out from what was written.
//------------------------------------------------------------------------------
// Input bytes per package. Deflate never grows data by more than a fraction of a
// percent, so a package stays clear of VirusTotal's 650 MB - the Mod Browser lists
// only packages VirusTotal knows - and of GitHub's asset limit alike; a single file
// over this still gets a package of its own.
static const u64 kPartInputLimit	= 512ull << 20;
static const u64 kAssetLimit		= 2048ull << 20;	// GitHub refuses a release asset of 2 GiB
static const u64 kIndexLimit		= 64ull << 20;		// 2.2: the game reads an index of 64 MiB at most
static const u32 kMaxPackages		= 64;
static const u32 kMaxAssetName		= 128;
static const u32 kZipMadeBy			= 63;				// FAT host, spec 6.3 - the one that defines UTF-8 names
static const u32 kZipUtf8Flag		= 1 << 11;

struct SStagedFile
{
	xr_string	rel;		// below the staged module folder, as found on disk
	xr_string	path;		// the same as the index states it: forward slashes, UTF-8
	xr_string	entry;		// zip entry name: modules/<id>/ + path
	xr_string	key;		// lowercased entry - the order of packages and index alike
	u64			size;
	FILETIME	written;
	// the index record, filled while the file is packed and read back afterwards
	xr_string	sha256;		// of the content
	u32			pack;		// 1-based package number, 0 = not read back yet
	u64			offset;		// first byte of the stored data inside the package
	u64			packed;		// length of that data
	u32			method;		// 0 stored, 8 raw deflate

				SStagedFile	() : size(0), written(), pack(0), offset(0), packed(0), method(0) {}
};

static void ListStagedFiles(LPCSTR dir, const xr_string& rel_dir, xr_vector<SStagedFile>& out)
{
	string_path mask;
	sprintf_s(mask, "%s\\*", dir);
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (INVALID_HANDLE_VALUE == h) return;
	do
	{
		if (!strcmp(fd.cFileName, ".") || !strcmp(fd.cFileName, "..")) continue;
		xr_string rel = rel_dir;
		if (!rel.empty()) rel += "\\";
		rel += fd.cFileName;
		if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)
		{
			string_path sub;
			sprintf_s(sub, "%s\\%s", dir, fd.cFileName);
			ListStagedFiles(sub, rel, out);
			continue;
		}
		SStagedFile f;
		f.rel		= rel;
		f.size		= (u64(fd.nFileSizeHigh) << 32) | fd.nFileSizeLow;
		f.written	= fd.ftLastWriteTime;
		out.push_back(f);
	} while (::FindNextFileA(h, &fd));
	::FindClose(h);
}

// MOD_UPDATES.md 2.2: what a path below modules/<id>/ may look like. The game
// refuses a whole package over one bad name, so the editor refuses first.
static bool PackagePathOk(const xr_string& path)
{
	for (size_t b = 0; b <= path.size();)
	{
		size_t e = path.find('/', b);
		if (e == xr_string::npos) e = path.size();
		xr_string part = path.substr(b, e - b);
		b = e + 1;

		if (part.empty() || part == "." || part == "..") return false;
		if (part[part.size() - 1] == '.' || part[part.size() - 1] == ' ') return false;
		for (size_t i = 0; i < part.size(); ++i)
		{
			const unsigned char c = (unsigned char)part[i];
			if (c < 0x20 || c == 0x7F || strchr("<>:\"|?*\\", c)) return false;
		}
		// device names are reserved with an extension too: nul, nul.txt
		const size_t dot = part.find('.');
		if (dot != xr_string::npos) part.erase(dot);
		for (size_t i = 0; i < part.size(); ++i)
			if (part[i] >= 'A' && part[i] <= 'Z') part[i] = char(part[i] - 'A' + 'a');
		if (part == "con" || part == "prn" || part == "aux" || part == "nul") return false;
		if (part.size() == 4 && (0 == part.compare(0, 3, "com") || 0 == part.compare(0, 3, "lpt")) &&
			part[3] >= '1' && part[3] <= '9') return false;
	}
	return true;
}

static uLong ZipDosTime(const FILETIME& utc)
{
	FILETIME local;
	WORD date = 0, time = 0;
	if (::FileTimeToLocalFileTime(&utc, &local) && ::FileTimeToDosDateTime(&local, &date, &time))
		return (uLong(date) << 16) | time;
	return 0x00210000;	// 1980-01-01: minizip reads 0 as "no date given"
}

// keeps the window alive during the long loops; true = the user cancelled
static bool PackagingAborted()
{
	if (!UI) return false;
	UI->ProgressCheckpoint();
	return UI->NeedAbort();
}

// One SHA-256 object for a whole packaging: Finish leaves it ready for the next
// input. Every path that abandons an input half-way ends the packaging as well.
struct SSha256
{
	BCRYPT_ALG_HANDLE	alg;
	BCRYPT_HASH_HANDLE	hash;

	SSha256() : alg(0), hash(0)
	{
		if (!BCRYPT_SUCCESS(::BCryptOpenAlgorithmProvider(&alg, BCRYPT_SHA256_ALGORITHM, NULL, 0)) ||
			!BCRYPT_SUCCESS(::BCryptCreateHash(alg, &hash, NULL, 0, NULL, 0, BCRYPT_HASH_REUSABLE_FLAG)))
			hash = 0;
	}
	~SSha256()
	{
		if (hash)	::BCryptDestroyHash(hash);
		if (alg)	::BCryptCloseAlgorithmProvider(alg, 0);
	}
	SSha256(const SSha256&) = delete;
	SSha256& operator=(const SSha256&) = delete;

	bool Add(const void* data, u32 size)
	{
		return hash && BCRYPT_SUCCESS(::BCryptHashData(hash, (PUCHAR)data, size, 0));
	}
	// lowercase hex
	bool Finish(xr_string& hex)
	{
		u8 digest[32];
		hex.clear();
		if (!hash || !BCRYPT_SUCCESS(::BCryptFinishHash(hash, digest, sizeof(digest), 0))) return false;
		for (int i = 0; i < (int)sizeof(digest); ++i)
		{
			char byte[4];
			sprintf_s(byte, "%02x", digest[i]);
			hex += byte;
		}
		return true;
	}
};

// files [first, last) of `files` into one zip. Every file is hashed on its way
// in - the index wants the SHA-256 of the content, and the content is read once.
// `done` counts input bytes across all packages, for the progress bar.
static bool ZipPackage(LPCSTR zip_path, LPCSTR module_dir, xr_vector<SStagedFile>& files,
					   u32 first, u32 last, xr_vector<u8>& buf, SSha256& sha, SPBItem* pb, u64& done, xr_string& err)
{
	zipFile zip = zipOpen64(zip_path, APPEND_STATUS_CREATE);
	if (!zip) { err = xr_string("can't create ") + zip_path; return false; }

	for (u32 i = first; i < last && err.empty(); ++i)
	{
		SStagedFile& f = files[i];
		string_path src;
		sprintf_s(src, "%s\\%s", module_dir, f.rel.c_str());
		HANDLE h = ::CreateFileA(src, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, NULL);
		if (INVALID_HANDLE_VALUE == h) { err = "can't read " + f.rel; break; }

		zip_fileinfo info = {};
		info.dosDate = ZipDosTime(f.written);
		// zip64 only where 32 bits cannot hold the entry (with room for data deflate
		// fails to shrink): every other entry stays readable by any unzip
		const int zip64 = (f.size >= 0xFF000000ull) ? 1 : 0;
		if (ZIP_OK != zipOpenNewFileInZip4_64(zip, f.entry.c_str(), &info, NULL, 0, NULL, 0, NULL,
				Z_DEFLATED, 9, 0, -MAX_WBITS, DEF_MEM_LEVEL, Z_DEFAULT_STRATEGY, NULL, 0,
				kZipMadeBy, HasNonAscii(f.entry.c_str()) ? kZipUtf8Flag : 0, zip64))
			err = "can't add " + f.rel + " to the package";

		if (pb) pb->Info(f.rel.c_str());
		u64 packed = 0;
		while (err.empty())
		{
			DWORD rd = 0;
			if (!::ReadFile(h, buf.data(), (DWORD)buf.size(), &rd, NULL)) { err = "can't read " + f.rel; break; }
			if (!rd) break;
			if (ZIP_OK != zipWriteInFileInZip(zip, buf.data(), rd)) { err = "can't write " + xr_string(zip_path); break; }
			if (!sha.Add(buf.data(), rd)) { err = "can't hash " + f.rel; break; }
			packed	+= rd;
			done	+= rd;
			if (pb) pb->Update(float(done >> 10));
			if (PackagingAborted()) err = "cancelled";
		}
		::CloseHandle(h);
		if (err.empty() && packed != f.size) err = "size changed while packaging: " + f.rel;
		if (err.empty() && !sha.Finish(f.sha256)) err = "can't hash " + f.rel;
		if (ZIP_OK != zipCloseFileInZip(zip) && err.empty()) err = "can't write " + xr_string(zip_path);
	}
	if (ZIP_OK != zipClose(zip, NULL) && err.empty()) err = xr_string("can't write ") + zip_path;
	if (err.empty()) return true;
	::DeleteFileA(zip_path);
	return false;
}

// Where every file of package `pack` really sits, asked of the FINISHED archive:
// the data offset comes from the local header minizip's reader finds there, the
// packed length and the method from its central directory. Nothing is assumed
// about what the writer did, so an index can only ever state what is on disk.
static bool ReadPackageLayout(LPCSTR zip_path, xr_vector<SStagedFile>& files, u32 first, u32 last,
							  u32 pack, xr_string& err)
{
	unzFile zip = unzOpen64(zip_path);
	if (!zip) { err = xr_string("can't read back ") + zip_path; return false; }

	xr_vector<char> name(0x10000);	// a zip name is 65535 bytes at most
	u32 found = 0;
	for (int res = unzGoToFirstFile(zip); err.empty(); res = unzGoToNextFile(zip))
	{
		if (UNZ_END_OF_LIST_OF_FILE == res) break;
		unz_file_info64 info;
		if (UNZ_OK != res || UNZ_OK != unzGetCurrentFileInfo64(zip, &info, name.data(), (uLong)name.size() - 1, NULL, 0, NULL, 0))
			{ err = xr_string("can't read back ") + zip_path; break; }
		name[info.size_filename < name.size() ? info.size_filename : name.size() - 1] = 0;

		// by name: the entry is looked up among the files this package was given
		xr_string key = name.data();
		for (size_t c = 0; c < key.size(); ++c)
			if (key[c] >= 'A' && key[c] <= 'Z') key[c] = char(key[c] - 'A' + 'a');
		xr_vector<SStagedFile>::iterator f = std::lower_bound(files.begin() + first, files.begin() + last, key,
			[](const SStagedFile& file, const xr_string& k) { return file.key < k; });
		if (f == files.begin() + last || f->entry != name.data() || f->pack)
			{ err = xr_string("package does not read back as written: ") + name.data(); break; }

		int method = 0, level = 0;
		if ((info.flag & 1) || (0 != info.compression_method && Z_DEFLATED != info.compression_method) ||
			info.uncompressed_size != f->size || UNZ_OK != unzOpenCurrentFile2(zip, &method, &level, 1))
			{ err = "package does not read back as written: " + f->rel; break; }
		f->offset	= unzGetCurrentFileZStreamPos64(zip);
		f->packed	= info.compressed_size;
		f->method	= (u32)info.compression_method;
		f->pack		= pack;
		unzCloseCurrentFile(zip);
		if (!f->offset) { err = "package does not read back as written: " + f->rel; break; }
		++found;
	}
	unzClose(zip);
	if (err.empty() && found != last - first) err = xr_string("package does not read back as written: ") + zip_path;
	return err.empty();
}

static bool Sha256File(LPCSTR path, xr_vector<u8>& buf, SSha256& sha, xr_string& hex, u64& size, xr_string& err)
{
	hex.clear();
	size = 0;
	HANDLE h = ::CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, NULL);
	if (INVALID_HANDLE_VALUE == h) { err = xr_string("can't read ") + path; return false; }

	bool ok = true;
	while (ok)
	{
		DWORD rd = 0;
		ok = !!::ReadFile(h, buf.data(), (DWORD)buf.size(), &rd, NULL);
		if (!ok || !rd) break;
		ok = sha.Add(buf.data(), rd);
		size += rd;
		if (PackagingAborted()) { err = "cancelled"; ok = false; }
	}
	::CloseHandle(h);
	if (!(ok && sha.Finish(hex)))
	{
		if (err.empty()) err = xr_string("can't hash ") + path;
		return false;
	}
	return true;
}

// <stem>.zip or <stem>.part<N>.zip - matched whole, so "test-1.0" never claims
// a package of "test-1.0.1" that shares the folder
static bool IsPackageName(LPCSTR name, const xr_string& stem)
{
	if (0 != _strnicmp(name, stem.c_str(), stem.size())) return false;
	const char* tail = name + stem.size();
	if (0 == _stricmp(tail, ".zip")) return true;
	if (0 != _strnicmp(tail, ".part", 5)) return false;
	tail += 5;
	if (*tail < '0' || *tail > '9') return false;
	while (*tail >= '0' && *tail <= '9') ++tail;
	return 0 == _stricmp(tail, ".zip");
}

// what a packaging of this id + version left in `dir`: the descriptor, the file
// index and the packages. These are the files the next packaging replaces.
static void ListReleaseFiles(LPCSTR dir, const xr_string& id, const xr_string& version, xr_vector<xr_string>& names)
{
	names.clear();
	const xr_string descriptor = id + ".update.ltx";
	const xr_string stem = id + "-" + version;
	const xr_string index = stem + ".files";
	string_path mask;
	sprintf_s(mask, "%s\\*", dir);
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (INVALID_HANDLE_VALUE == h) return;
	do
	{
		if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) continue;
		if (0 == _stricmp(fd.cFileName, descriptor.c_str()) || 0 == _stricmp(fd.cFileName, index.c_str()) ||
			IsPackageName(fd.cFileName, stem))
			names.push_back(xr_string(fd.cFileName));
	} while (::FindNextFileA(h, &fd));
	::FindClose(h);
}

// A release is rebuilt from the module every time, so the files it replaces are
// deleted outright - the editor keeps nothing in the Recycle Bin.
static void DeleteOutright(LPCSTR path)
{
	::SetFileAttributesA(path, FILE_ATTRIBUTE_NORMAL);
	::DeleteFileA(path);
}

static DWORD CALLBACK MoveProgress(LARGE_INTEGER, LARGE_INTEGER, LARGE_INTEGER, LARGE_INTEGER,
								   DWORD, DWORD, HANDLE, HANDLE, LPVOID)
{
	if (UI) UI->ProgressCheckpoint();
	return PROGRESS_CONTINUE;
}

// everything that decides whether this manifest can be released at all
static bool ReleaseGate(const EditorMod::SManifest& m, xr_string& err)
{
	if (!EditorMod::ValidateId(m.id.c_str()))
		err = "module id missing or invalid in mod.ltx (allowed: a-z 0-9 _ . -)";
	else if (!EditorMod::ValidateVersion(m.version.c_str()))
		err = "a release needs an orderable version: 1-4 numbers separated by dots, e.g. 1.2.0";
	else if (!EditorMod::ValidateWebsite(m.website.c_str()))
		err = "invalid website in mod.ltx: only https://ap-pro.ru or https://moddb.com links are allowed";
	else if (!EditorMod::ValidateGithubRepo(m.update_github.c_str()))
		err = "invalid [update] github in mod.ltx: owner/repo expected";
	// 2.1: an asset name - package or index - is 1-128 characters of a charset id
	// and version stay inside, and does not start with a dot. The longest name a
	// release can need is the last part's; "<id>-<version>.files" is shorter.
	else if (m.id[0] == '.')
		err = "module id starts with a dot - a release asset name cannot";
	else if (m.id.size() + m.version.size() + xr_strlen("-.part64.zip") > kMaxAssetName)
		err = "module id and version are too long for a release asset name (128 characters)";
	return err.empty();
}

// "<project>\releases\<version>" by default; a relative folder is taken against
// the project, never the process CWD. Inside the project only releases\ is
// allowed: anywhere else the packages would ship inside the next build.
static bool ResolveReleaseDir(LPCSTR project_root, LPCSTR out_dir, const xr_string& version,
							  string_path& dst, xr_string& err)
{
	string_path asked = {}, joined;
	NormalizePath(out_dir ? out_dir : "", asked, sizeof(asked));
	const bool absolute = (asked[0] && asked[1] == ':') || (asked[0] == '\\' && asked[1] == '\\');
	if (!asked[0])		sprintf_s(joined, "%s\\releases\\%s", project_root, version.c_str());
	else if (absolute)	strcpy_s(joined, asked);
	else				sprintf_s(joined, "%s\\%s", project_root, asked);
	// resolves ".." too, so the check below looks at where the folder really is
	string_path root;
	if (!::GetFullPathNameA(joined, sizeof(dst), dst, NULL) ||
		!::GetFullPathNameA(project_root, sizeof(root), root, NULL)) { err = "invalid output folder"; return false; }
	size_t n = xr_strlen(root);
	while (n && root[n - 1] == '\\') root[--n] = 0;

	if (0 != _strnicmp(dst, root, n) || (dst[n] != '\\' && dst[n] != 0)) return true;
	if (0 == _strnicmp(dst + n, "\\releases", 9) && (dst[n + 9] == '\\' || dst[n + 9] == 0)) return true;
	err = "output folder is inside the project content - use <project>\\releases or a folder outside the project";
	return false;
}

// The Mod Browser's content rules (MOD_CATALOG 8, 10.3) over what the release ships, read the
// way the catalog reads a release: a script whole, anything else by what it starts with.
static void CheckStagedContent(const xr_string& module_dir, const xr_vector<SStagedFile>& files,
							   EditorMod::SPackageResult& result, xr_string* warnings)
{
	xr_vector<EditorCatalog::SFinding> findings;
	xr_vector<char> data;
	for (u32 i = 0; i < files.size(); ++i)
	{
		const SStagedFile& f = files[i];
		string_path src;
		sprintf_s(src, "%s\\%s", module_dir.c_str(), f.rel.c_str());
		HANDLE h = ::CreateFileA(src, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, NULL);
		if (INVALID_HANDLE_VALUE == h) continue;
		size_t got = 0;
		for (size_t want = EditorCatalog::ContentHead(f.path.c_str(), (size_t)f.size); want > got;
			 want = EditorCatalog::ContentNeed(data.data(), got, (size_t)f.size))
		{
			data.resize(want);
			DWORD read = 0;
			if (!::ReadFile(h, data.data() + got, (DWORD)(want - got), &read, NULL) || !read) break;
			got += read;
		}
		::CloseHandle(h);
		EditorCatalog::CheckContent(f.path.c_str(), got ? data.data() : "", got, findings);
	}
	for (u32 i = 0; i < findings.size(); ++i)
		(EditorCatalog::lvBlock == findings[i].level ? result.catalog_block : result.catalog_review).push_back(findings[i].text);
	if (!warnings) return;
	// the author learns it here, not from a refused submission
	for (u32 i = 0; i < result.catalog_block.size() && i < 10; ++i)
		*warnings += "the Mod Browser refuses " + result.catalog_block[i] + "\n";
	if (result.catalog_block.size() > 10)
		*warnings += NqUtil::Format("the Mod Browser refuses %u more file(s)\n", u32(result.catalog_block.size() - 10));
	if (!result.catalog_review.empty())
		*warnings += NqUtil::Format("the Mod Browser's review looks at %u place(s) in the scripts, first: %s\n",
			(u32)result.catalog_review.size(), result.catalog_review[0].c_str());
}

// stage -> packages -> file index -> descriptor. `stage` is a private scratch
// folder the caller creates and removes, so a failure leaves `out` untouched.
static bool BuildRelease(LPCSTR project_root, const EditorMod::SManifest& m, LPCSTR stage, LPCSTR out,
						 LPCSTR requires_game, EditorMod::SPackageResult& result, xr_string& err, xr_string* warnings)
{
	// The real module export does the staging: what the release ships is what a
	// build ships, and whatever a build refuses (no target mode, a broken quest)
	// is refused here by the same code.
	int exported = 0;
	xr_string module_dir;
	if (!EditorMod::Export(project_root, stage, false, exported, module_dir, err, true, warnings, &result.left_out))
		return false;

	xr_vector<SStagedFile> files;
	ListStagedFiles(module_dir.c_str(), "", files);
	const xr_string prefix = "modules/" + m.id + "/";
	bool has_manifest = false;
	u64 unpacked = 0;
	for (u32 i = 0; i < files.size(); ++i)
	{
		SStagedFile& f = files[i];
		xr_string rel = AnsiToUtf8(f.rel.c_str());
		for (size_t c = 0; c < rel.size(); ++c)
			if (rel[c] == '\\') rel[c] = '/';
		if (!PackagePathOk(rel))
		{
			err = "file name the game's updater refuses (reserved name, trailing dot or space, control character): " + f.rel;
			return false;
		}
		// an index record is space-separated and the path is the rest of its line
		if (rel[0] == ' ')
		{
			err = "file name starts with a space - the file index cannot state it safely: " + f.rel;
			return false;
		}
		f.path = rel;
		f.entry = prefix + rel;
		f.key = f.entry;
		for (size_t c = 0; c < f.key.size(); ++c)
			if (f.key[c] >= 'A' && f.key[c] <= 'Z') f.key[c] = char(f.key[c] - 'A' + 'a');
		has_manifest = has_manifest || f.key == prefix + "mod.ltx";
		unpacked += f.size;
	}
	if (!has_manifest) { err = "the staged module has no mod.ltx"; return false; }
	CheckStagedContent(module_dir, files, result, warnings);
	// the same input always yields the same package order
	std::sort(files.begin(), files.end(), [](const SStagedFile& a, const SStagedFile& b) { return a.key < b.key; });

	// [first, last) of every package; a file over the limit ends up alone in its own
	xr_vector<std::pair<u32, u32> > parts;
	u64 part_bytes = 0;
	u32 part_first = 0;
	for (u32 i = 0; i < files.size(); ++i)
	{
		if (i > part_first && part_bytes + files[i].size > kPartInputLimit)
		{
			parts.push_back(std::make_pair(part_first, i));
			part_first = i;
			part_bytes = 0;
		}
		part_bytes += files[i].size;
	}
	parts.push_back(std::make_pair(part_first, (u32)files.size()));
	if (parts.size() > kMaxPackages)
	{
		err = NqUtil::Format("the module needs %u packages - a release holds %u at most", (u32)parts.size(), kMaxPackages);
		return false;
	}

	string_path packages_dir, path;
	sprintf_s(packages_dir, "%s\\packages", stage);
	CreateDirChain(packages_dir);
	SSha256 sha;
	if (!sha.hash) { err = "SHA-256 is not available on this system"; return false; }

	// on screen until the function returns: moving gigabytes to another drive
	// is part of the wait
	struct SProgress
	{
		SPBItem* item;
		~SProgress() { if (item) UI->ProgressEnd(item); }
	} progress = { UI ? UI->ProgressStart(float(unpacked >> 10) + 1.f, "Packaging release...") : 0 };
	SPBItem* const pb = progress.item;
	xr_vector<u8> buf(1 << 20);
	u64 done = 0, packed = 0;
	for (u32 p = 0; p < parts.size() && err.empty(); ++p)
	{
		EditorMod::SPackageFile pkg;
		pkg.name = m.id + "-" + m.version;
		if (parts.size() > 1) pkg.name += NqUtil::Format(".part%u", p + 1);
		pkg.name += ".zip";
		sprintf_s(path, "%s\\%s", packages_dir, pkg.name.c_str());
		if (!ZipPackage(path, module_dir.c_str(), files, parts[p].first, parts[p].second, buf, sha, pb, done, err)) break;
		if (pb) pb->Info(pkg.name.c_str());
		if (!ReadPackageLayout(path, files, parts[p].first, parts[p].second, p + 1, err)) break;
		if (!Sha256File(path, buf, sha, pkg.sha256, pkg.size, err)) break;
		if (pkg.size >= kAssetLimit)
		{
			u32 biggest = parts[p].first;
			for (u32 i = parts[p].first; i < parts[p].second; ++i)
				if (files[i].size > files[biggest].size) biggest = i;
			err = NqUtil::Format("package %s is %llu bytes - GitHub refuses a release asset of 2 GiB and more (largest file inside: %s)",
				pkg.name.c_str(), pkg.size, files[biggest].rel.c_str());
			break;
		}
		packed += pkg.size;
		result.packages.push_back(pkg);
	}
	if (!err.empty()) return false;

	// The file index (2.2). `pack` lines follow the descriptor's [packages] lines,
	// `file` lines the package order; LF line ends. Nothing in it depends on when
	// the packaging ran, so the same module always yields the same bytes.
	xr_string index_text = "xms-files 1\n";
	for (u32 p = 0; p < result.packages.size(); ++p)
		index_text += "pack " + result.packages[p].name + "\n";
	for (u32 i = 0; i < files.size(); ++i)
	{
		const SStagedFile& f = files[i];
		index_text += NqUtil::Format("file %s %llu %u %llu %llu %u ", f.sha256.c_str(), f.size, f.pack, f.offset, f.packed, f.method);
		index_text += f.path;
		index_text += "\n";
	}
	if (index_text.size() > kIndexLimit)
	{
		err = NqUtil::Format("the file index is %llu bytes - the game reads 64 MiB at most", (u64)index_text.size());
		return false;
	}
	result.index.name = m.id + "-" + m.version + ".files";
	result.index.size = index_text.size();
	if (!sha.Add(index_text.data(), (u32)index_text.size()) || !sha.Finish(result.index.sha256))
		{ err = "can't hash the file index"; return false; }

	// Only now is the output folder touched. What an earlier packaging of this id
	// and version left there goes first - a 3-part build followed by a 1-part one
	// must not leave part2 and part3 beside the new release.
	CreateDirChain(out);
	if (!DirExists(out)) { err = xr_string("can't create the output folder ") + out; return false; }
	xr_vector<xr_string> old_files;
	ListReleaseFiles(out, m.id, m.version, old_files);
	for (u32 i = 0; i < old_files.size(); ++i)
	{
		sprintf_s(path, "%s\\%s", out, old_files[i].c_str());
		DeleteOutright(path);
	}
	for (u32 p = 0; p < result.packages.size(); ++p)
	{
		string_path from;
		sprintf_s(from, "%s\\%s", packages_dir, result.packages[p].name.c_str());
		sprintf_s(path, "%s\\%s", out, result.packages[p].name.c_str());
		if (pb) pb->Info(path);
		if (!::MoveFileWithProgressA(from, path, &MoveProgress, NULL, MOVEFILE_COPY_ALLOWED | MOVEFILE_REPLACE_EXISTING))
			{ err = xr_string("can't write ") + path; return false; }
	}
	sprintf_s(path, "%s\\%s", out, result.index.name.c_str());
	if (!WriteTextFile(path, index_text.c_str())) { err = xr_string("can't write ") + path; return false; }

	// the descriptor goes last: it is the file that vouches for the other two kinds
	xr_string text = "; XMS module release descriptor. Generated by XFined Editor.\r\n[release]\r\n";
	AppendKey(text, "schema", "1", 13);
	AppendKey(text, "id", m.id.c_str(), 13);
	AppendKey(text, "version", m.version.c_str(), 13);
	if (requires_game[0])
		AppendKey(text, "requires_game", requires_game, 13);
	AppendKey(text, "files", NqUtil::Format("%u", (u32)files.size()).c_str(), 13);
	AppendKey(text, "unpacked", NqUtil::Format("%llu", unpacked).c_str(), 13);
	AppendKey(text, "index", (result.index.name + NqUtil::Format(", %llu, ", result.index.size) + result.index.sha256).c_str(), 13);
	text += "\r\n[packages]\r\n";
	for (u32 p = 0; p < result.packages.size(); ++p)
		text += result.packages[p].name + NqUtil::Format(" = %llu, ", result.packages[p].size) + result.packages[p].sha256 + "\r\n";

	// The Mod Browser's card: what the browser shows before the module is installed, the
	// manifest and the thumbnail pinned by their hashes (MOD_CATALOG 4, 6.1)
	EditorCatalog::SCardFile manifest_ref, thumbnail_ref;
	bool has_thumbnail = false;
	xr_string thumbnail_path = m.thumbnail;
	for (size_t c = 0; c < thumbnail_path.size(); ++c)
		if (thumbnail_path[c] == '\\') thumbnail_path[c] = '/';
	for (u32 i = 0; i < files.size(); ++i)
	{
		EditorCatalog::SCardFile* ref = files[i].path == "mod.ltx" ? &manifest_ref :
			(!thumbnail_path.empty() && 0 == _stricmp(files[i].path.c_str(), thumbnail_path.c_str())) ? &thumbnail_ref : 0;
		if (!ref) continue;
		ref->path = files[i].path;
		ref->size = files[i].size;
		ref->sha256 = files[i].sha256;
		has_thumbnail |= ref == &thumbnail_ref;
	}
	if (!thumbnail_path.empty() && warnings)
	{
		xr_string bytes;
		string_path picture;
		sprintf_s(picture, "%s\\%s", module_dir.c_str(), m.thumbnail.c_str());
		if (!has_thumbnail)
			*warnings += "the thumbnail " + m.thumbnail + " is not in the build - the Mod Browser shows no picture\n";
		else if (!ReadTextFile(picture, bytes) || !EditorCatalog::ValidThumbnail(bytes.data(), bytes.size()))
		{
			*warnings += "the thumbnail " + m.thumbnail + " is not a 256 x 256 DXT1/DXT5 DDS - the Mod Browser shows no picture\n";
			has_thumbnail = false;
		}
	}
	text += EditorCatalog::CardSection(project_root, m, manifest_ref, has_thumbnail ? &thumbnail_ref : 0);

	// signed with the author key when there is one to sign with: one signature covers the
	// card, the index hash and the package hashes - every byte of the release
	const EditorCatalog::SKeyState key = EditorCatalog::KeyState();
	if (key.exists && key.error.empty() && !key.locked)
	{
		xr_string signed_text, sign_err;
		if (!EditorCatalog::Sign(text, signed_text, sign_err))
			{ err = "the release could not be signed: " + sign_err; return false; }
		text = signed_text;
		result.signed_ = true;
		result.key_id = key.key_id;
	}
	else if (warnings)
		*warnings += key.exists ? (key.locked ? "the release is not signed: the author key is locked - give its password first\n" :
											  "the release is not signed: " + key.error + "\n") :
			xr_string("the release is not signed - the Mod Browser takes signed releases only (Mod > Submit to Mod Browser sets up the author key)\n");

	sprintf_s(path, "%s\\%s.update.ltx", out, m.id.c_str());
	if (!WriteTextFile(path, text.c_str())) { err = xr_string("can't write ") + path; return false; }
	// a publisher checks every uploaded asset against these, the descriptor included
	result.descriptor_file.name = m.id + ".update.ltx";
	result.descriptor_file.size = text.size();
	if (!sha.Add(text.data(), (u32)text.size()) || !sha.Finish(result.descriptor_file.sha256))
		{ err = "can't hash the release descriptor"; return false; }

	result.out_dir			= out;
	result.descriptor		= path;
	result.requires_game	= requires_game;
	result.files			= (int)files.size();
	result.unpacked			= unpacked;
	result.packed			= packed;
	Msg("* [XMS] release %s %s: %d file(s), %llu byte(s) in %u package(s) -> %s",
		m.id.c_str(), m.version.c_str(), result.files, packed, (u32)result.packages.size(), out);
	return true;
}

// A crashed packaging leaves its stage - a full copy of the module plus the
// zips - in a folder the content browser never shows. Stages whose editor is
// gone are removed; one that belongs to a live process may be at work.
static void RemoveDeadStages(LPCSTR temp_dir)
{
	string_path mask;
	sprintf_s(mask, "%s\\release_stage_*", temp_dir);
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (INVALID_HANDLE_VALUE == h) return;
	do
	{
		if (!(fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)) continue;
		const DWORD pid = (DWORD)strtoul(fd.cFileName + xr_strlen("release_stage_"), 0, 10);
		if (HANDLE owner = pid ? ::OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) : 0)
		{
			::CloseHandle(owner);
			continue;
		}
		string_path dead;
		sprintf_s(dead, "%s\\%s", temp_dir, fd.cFileName);
		WipeTree(dead);
		::RemoveDirectoryA(dead);
	} while (::FindNextFileA(h, &fd));
	::FindClose(h);
}

bool EditorMod::PackageRelease(LPCSTR project_root, LPCSTR out_dir, LPCSTR requires_game,
							   SPackageResult& result, xr_string& err, xr_string* warnings)
{
	result = SPackageResult();
	err.clear();
	if (warnings) warnings->clear();
	if (!project_root || !project_root[0])	{ err = "no active project"; return false; }
	if (!requires_game) requires_game = "";

	SManifest m;
	if (!Load(project_root, m))				{ err = "mod.ltx not found in the project"; return false; }
	if (!ReleaseGate(m, err))				return false;
	if (requires_game[0] && !DottedNumber(requires_game, 3, 3))
		{ err = "invalid requires_game: MAJOR.MINOR.PATCH expected, e.g. 1.4.1"; return false; }

	string_path out, stage;
	if (!ResolveReleaseDir(project_root, out_dir, m.version, out, err)) return false;

	// temp\ is editor-only scratch that no export ever reads, so the stage can sit
	// inside the project; the pid keeps two editors on one project apart
	sprintf_s(stage, "%s\\temp", project_root);
	RemoveDeadStages(stage);
	sprintf_s(stage, "%s\\temp\\release_stage_%u", project_root, (u32)::GetCurrentProcessId());
	CreateDirChain(stage);
	if (!DirExists(stage))					{ err = "can't create the staging folder in the project's temp"; return false; }
	WipeTree(stage);

	const bool ok = BuildRelease(project_root, m, stage, out, requires_game, result, err, warnings);
	WipeTree(stage);
	::RemoveDirectoryA(stage);
	if (!ok) result = SPackageResult();
	return ok;
}

void EditorMod::ListPackagedFiles(LPCSTR project_root, LPCSTR out_dir, LPCSTR id, LPCSTR version,
								  xr_vector<xr_string>& names)
{
	names.clear();
	string_path dir;
	xr_string ignored;
	if (project_root && project_root[0] && ValidateId(id) && ValidateVersion(version) &&
		ResolveReleaseDir(project_root, out_dir, version, dir, ignored))
		ListReleaseFiles(dir, id, version, names);
}

bool EditorMod::LinkedGameVersion(xr_string& out)
{
	out.clear();
	LPCSTR game = EditorProject::GameRoot();
	if (!game || !game[0]) return false;
	string_path exe;
	sprintf_s(exe, "%s\\xrEngine.exe", game);

	DWORD unused = 0;
	const DWORD size = ::GetFileVersionInfoSizeA(exe, &unused);
	if (!size) return false;
	xr_vector<u8> data(size);
	VS_FIXEDFILEINFO* info = 0;
	UINT info_size = 0;
	if (!::GetFileVersionInfoA(exe, 0, size, data.data()) ||
		!::VerQueryValueA(data.data(), "\\", (LPVOID*)&info, &info_size) || !info || info_size < sizeof(*info))
		return false;
	out = NqUtil::Format("%u.%u.%u", (u32)HIWORD(info->dwFileVersionMS), (u32)LOWORD(info->dwFileVersionMS),
		(u32)HIWORD(info->dwFileVersionLS));
	return true;
}

//------------------------------------------------------------------------------
// the Mod Browser's picture and tags
//------------------------------------------------------------------------------
static const char* kThumbnailName = "thumbnail.dds";

// a picture on its way into the project waits in the editor's scratch
static xr_string PendingThumbnailPath(LPCSTR project_root)
{
	return xr_string(project_root) + "\\temp\\thumbnail_pending.dds";
}

static bool ConvertThumbnail(LPCSTR project_root, LPCSTR picture, xr_string& pending, xr_string& err)
{
	pending = PendingThumbnailPath(project_root);
	CreateDirChain((xr_string(project_root) + "\\temp").c_str());
	return EditorCatalog::MakeThumbnail(picture, pending.c_str(), err);
}

static bool PlaceThumbnail(LPCSTR project_root, const xr_string& pending, xr_string& err)
{
	string_path target;
	sprintf_s(target, "%s\\%s", project_root, kThumbnailName);
	::SetFileAttributesA(target, FILE_ATTRIBUTE_NORMAL);
	if (::MoveFileExA(pending.c_str(), target, MOVEFILE_REPLACE_EXISTING | MOVEFILE_COPY_ALLOWED)) return true;
	err = xr_string("can't write ") + target;
	return false;
}

// Only the file the editor makes goes with the key: a thumbnail the author pointed at
// elsewhere in the module is theirs.
static bool OwnThumbnail(const xr_string& thumbnail) { return 0 == _stricmp(thumbnail.c_str(), kThumbnailName); }

bool EditorMod::ApplyThumbnail(LPCSTR project_root, LPCSTR value, SManifest& m, xr_string& err)
{
	err.clear();
	xr_string v = value ? value : "";
	Trim(v);
	for (size_t i = 0; i < v.size(); ++i)
		if (v[i] == '/') v[i] = '\\';
	if (v.empty())
	{
		if (OwnThumbnail(m.thumbnail))
		{
			string_path path;
			sprintf_s(path, "%s\\%s", project_root, kThumbnailName);
			DeleteOutright(path);
		}
		m.thumbnail.clear();
		return true;
	}
	const bool absolute = v.size() > 2 && (v[1] == ':' || (v[0] == '\\' && v[1] == '\\'));
	const xr_string full = absolute ? v : xr_string(project_root) + "\\" + v;
	const size_t root_len = xr_strlen(project_root);
	if (NqUtil::Extension(full.c_str()) == ".dds")
	{
		// a ready thumbnail inside the project is named as it is
		xr_string bytes;
		if (full.size() <= root_len + 1 || 0 != _strnicmp(full.c_str(), project_root, root_len) || full[root_len] != '\\')
			{ err = "a .dds thumbnail has to be a file of the project: " + v; return false; }
		if (!ReadTextFile(full.c_str(), bytes) || !EditorCatalog::ValidThumbnail(bytes.data(), bytes.size()))
			{ err = v + " is not a 256 x 256 DXT1/DXT5 DDS (one mip level or all nine)"; return false; }
		m.thumbnail = full.substr(root_len + 1);
		return true;
	}
	xr_string pending;
	if (!ConvertThumbnail(project_root, full.c_str(), pending, err) || !PlaceThumbnail(project_root, pending, err))
		return false;
	m.thumbnail = kThumbnailName;
	return true;
}

bool EditorMod::ApplyTags(LPCSTR value, SManifest& m, xr_string& err)
{
	err.clear();
	xr_vector<xr_string> tags, unique;
	SplitList(tags, value ? value : "", ',');
	for (u32 i = 0; i < tags.size(); ++i)
	{
		for (size_t c = 0; c < tags[i].size(); ++c) tags[i][c] = char(tolower((unsigned char)tags[i][c]));
		if (!EditorCatalog::KnownTag(tags[i].c_str()))
		{
			err = "unknown tag '" + tags[i] + "' (allowed:";
			for (u32 k = 0; k < EditorCatalog::TagVocabulary().size(); ++k)
				err += (k ? ", " : " ") + EditorCatalog::TagVocabulary()[k];
			err += ")";
			return false;
		}
		if (std::find(unique.begin(), unique.end(), tags[i]) == unique.end()) unique.push_back(tags[i]);
	}
	m.tags = unique;
	return true;
}

//------------------------------------------------------------------------------
// UI: the Mod Settings tab, the build dialogs and the results they report
//------------------------------------------------------------------------------
using ModToken = XFinedTheme::ColorToken;

static const char* kTitleExport		= EditorUiContract::kDialogBuildToFolder;
static const char* kTitleFlat		= EditorUiContract::kDialogExportFlat;
static const char* kTitlePackage	= EditorUiContract::kDialogPackageRelease;

static xr_string CountText(int n, LPCSTR one, LPCSTR many)
{
	return n == 1 ? NqUtil::Format("1 %s", one) : NqUtil::Format("%d %s", n, many);
}

// the lines of a text, trimmed, empty ones dropped
static void SplitLines(const xr_string& text, XUI::Strings& out)
{
	xr_vector<xr_string> lines;
	SplitList(lines, text.c_str(), '\n');
	out.insert(out.end(), lines.begin(), lines.end());
}

static void OpenFolder(const xr_string& path)
{
	if (!path.empty())
		::ShellExecuteA(NULL, "open", path.c_str(), NULL, NULL, SW_SHOWNORMAL);
}

//------------------------------------------------------------------------------
// the Mod Browser's picture
//------------------------------------------------------------------------------
static bool					s_SettingsOpen		= false;
static xr_string			s_SettingsThumb;				// [module] thumbnail as the form loaded it
// Nothing reaches the project before Save: a chosen picture waits in temp\, a removal is only a mark.
static xr_string			s_ThumbPending;				// the converted picture, "" = none
static bool					s_ThumbRemove		= false;
static bool					s_ThumbPickQueued	= false;
static xr_string			s_ThumbError;
static void*				s_ThumbTexture		= 0;		// the preview
static xr_string			s_ThumbTextureOf;				// the file it shows

// drops the picture nobody saved and the preview; the tab is closed or reloaded
static void ThumbnailCleanup()
{
	if (!s_ThumbPending.empty()) ::DeleteFileA(s_ThumbPending.c_str());
	s_ThumbPending.clear();
	s_ThumbRemove = false;
	s_ThumbError.clear();
	XFinedMCP::ReleaseTexture(s_ThumbTexture);
	s_ThumbTexture = 0;
	s_ThumbTextureOf.clear();
}

// the picture dialog is a native window: it runs outside the ImGui pass
static void PickThumbnail()
{
	s_ThumbPickQueued = false;
	if (!s_SettingsOpen || !EditorProject::Active()) return;
	wchar_t picked[MAX_PATH] = {};
	const HRESULT co = ::CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
	IFileOpenDialog* dlg = 0;
	if (SUCCEEDED(::CoCreateInstance(CLSID_FileOpenDialog, NULL, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&dlg))))
	{
		const COMDLG_FILTERSPEC types[] = { { L"Pictures (PNG, JPG, TGA, BMP)", L"*.png;*.jpg;*.jpeg;*.tga;*.bmp" } };
		dlg->SetFileTypes(1, types);
		dlg->SetTitle(L"Choose the Mod Browser picture");
		DWORD opts = 0;
		dlg->GetOptions(&opts);
		dlg->SetOptions(opts | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST);
		IShellItem* item = 0;
		if (SUCCEEDED(dlg->Show(EDevice ? EDevice->m_hWnd : NULL)) && SUCCEEDED(dlg->GetResult(&item)))
		{
			PWSTR w = 0;
			if (SUCCEEDED(item->GetDisplayName(SIGDN_FILESYSPATH, &w)))
			{
				wcsncpy_s(picked, w, _TRUNCATE);
				::CoTaskMemFree(w);
			}
			item->Release();
		}
		dlg->Release();
	}
	if (co == S_OK) ::CoUninitialize();
	if (!picked[0]) return;

	// the process speaks UTF-8 (LevelEditor.manifest): the path goes to the loader as it is
	char path[MAX_PATH * 3] = {};
	::WideCharToMultiByte(CP_UTF8, 0, picked, -1, path, sizeof(path), 0, 0);
	xr_string pending, err;
	if (ConvertThumbnail(EditorProject::Root(), path, pending, err))
	{
		s_ThumbPending = pending;
		s_ThumbRemove = false;
		s_ThumbError.clear();
		// the preview reloads: the scratch file has the same name as the last one
		s_ThumbTextureOf.clear();
	}
	else
		s_ThumbError = err;
}

//------------------------------------------------------------------------------
// Mod Settings: mod.ltx as a form. The form is compared field by field with what
// it was loaded from, so a save elsewhere (World Rules, Exclude from Build, MCP)
// reaches every field the author has not touched and none that they have.
//------------------------------------------------------------------------------
struct SSettingsForm
{
	char	id[128];
	char	name[256];
	char	version[64];
	char	author[256];
	char	description[4096];
	// wider than the 512 bytes a website may have, so a longer one is flagged, not cut
	char	website[1024];
	char	github[512];
	char	target[32];
	char	mode[128];
	char	pmode_id[128];			// the first declared mode; further ones survive untouched
	char	pmode_title[256];
	char	requires_[2048];
	char	after[1024];
	char	before[1024];
	char	conflicts[2048];
	char	exclude[4096];
	char	tags[512];				// comma list
	char	pda[MAX_PATH * 2];
};

struct SFormField { size_t offset, size; };
static const SFormField kFormFields[] =
{
	{ offsetof(SSettingsForm, id),			sizeof(SSettingsForm::id) },
	{ offsetof(SSettingsForm, name),		sizeof(SSettingsForm::name) },
	{ offsetof(SSettingsForm, version),		sizeof(SSettingsForm::version) },
	{ offsetof(SSettingsForm, author),		sizeof(SSettingsForm::author) },
	{ offsetof(SSettingsForm, description),	sizeof(SSettingsForm::description) },
	{ offsetof(SSettingsForm, website),		sizeof(SSettingsForm::website) },
	{ offsetof(SSettingsForm, github),		sizeof(SSettingsForm::github) },
	{ offsetof(SSettingsForm, target),		sizeof(SSettingsForm::target) },
	{ offsetof(SSettingsForm, mode),		sizeof(SSettingsForm::mode) },
	{ offsetof(SSettingsForm, pmode_id),	sizeof(SSettingsForm::pmode_id) },
	{ offsetof(SSettingsForm, pmode_title),	sizeof(SSettingsForm::pmode_title) },
	{ offsetof(SSettingsForm, requires_),	sizeof(SSettingsForm::requires_) },
	{ offsetof(SSettingsForm, after),		sizeof(SSettingsForm::after) },
	{ offsetof(SSettingsForm, before),		sizeof(SSettingsForm::before) },
	{ offsetof(SSettingsForm, conflicts),	sizeof(SSettingsForm::conflicts) },
	{ offsetof(SSettingsForm, exclude),		sizeof(SSettingsForm::exclude) },
	{ offsetof(SSettingsForm, tags),		sizeof(SSettingsForm::tags) },
	{ offsetof(SSettingsForm, pda),			sizeof(SSettingsForm::pda) },
};

enum ESettingsPage { pgGeneral = 0, pgBrowser, pgMode, pgRelations, pgContents, pgPda, pgCount };
struct SSettingsPage { LPCSTR id; LPCSTR label; XUI::Icon icon; LPCSTR about; };
static const SSettingsPage kSettingsPages[pgCount] =
{
	{ "general",		"General",			XUI::Icon::Info,			"What the game's Mods menu shows, and where the game looks for updates." },
	{ "mod_browser",	"Mod Browser",		XUI::Icon::Globe,			"Shown next to the module in the game's Mods window and Mod Browser. Listing it there: Mod > Submit to Mod Browser." },
	{ "game_mode",		"Game Mode",		XUI::Icon::Mode,			"A module targets one mode: the plain game, a mode the linked game has, or a new mode it brings. A mode is not the mod: the mod's name is on General." },
	{ "relations",		"Relations",		XUI::Icon::Relations,		"The modules this one needs, loads around or cannot run with. Only matters when the player runs several modules at once: a standalone mod leaves all four empty." },
	{ "build_contents",	"Build Contents",	XUI::Icon::Exclude,			"The project folder is your working place as well as the module. The build ships everything in it except what is listed here." },
	{ "pda",			"PDA",				XUI::Icon::PdaInterface,	"A module can give the PDA screens of its own. The layout named here is the one the game shows." },
};

struct SSettingsProblem
{
	int			page;
	LPCSTR		field;		// the row it is shown under
	XUI::Tone	tone;
	bool		blocks;		// Save waits until it is fixed
	xr_string	text;
};

static SSettingsForm				s_Form;				// what the tab shows
static SSettingsForm				s_FormBase;			// what it was loaded from
static u32							s_FormGen		= u32(-1);
static xr_string					s_FormRoot;
static int							s_SettingsPage	= pgGeneral;
static xr_string					s_SettingsStatus;	// the last save's failure
static xr_vector<SSettingsProblem>	s_SettingsProblems;	// this frame's, for the rows, the nav, the footer and MCP
// taken when the tab opens, never per frame: counting files and reading the game's layout is disk work
static int							s_GatedFiles	= 0;
static int							s_GatedPatches	= 0;
static bool							s_SlotChecked	= false;
static bool							s_SlotOk		= false;
static xr_string					s_SlotError;
static xr_vector<xr_string>			s_PdaFiles;			// project-relative .pdaasset files

// the modules installed in the linked game, for the Relations pickers; read when that page is shown
struct SInstalledModule
{
	xr_string	id;		// lower case, as the game compares it
	xr_string	name;
};
static xr_vector<SInstalledModule>	s_Installed;
static bool							s_InstalledScanned	= false;
static char							s_ModuleSearch[64]	= {};

static char* FormField(SSettingsForm& f, const SFormField& field) { return (char*)&f + field.offset; }

static void FormFromManifest(const EditorMod::SManifest& m, SSettingsForm& f)
{
	ZeroMemory(&f, sizeof(f));
	strncpy_s(f.id, m.id.c_str(), _TRUNCATE);
	strncpy_s(f.name, m.name.c_str(), _TRUNCATE);
	strncpy_s(f.version, m.version.c_str(), _TRUNCATE);
	strncpy_s(f.author, m.author.c_str(), _TRUNCATE);
	// A hand-written text longer than the box is cut at a character boundary:
	// one broken UTF-8 byte makes the game read the whole file as cp1251.
	size_t cut = m.description.size();
	if (cut >= sizeof(f.description))
	{
		cut = sizeof(f.description) - 1;
		while (cut && ((unsigned char)m.description[cut] & 0xC0) == 0x80) --cut;
	}
	strncpy_s(f.description, m.description.substr(0, cut).c_str(), _TRUNCATE);
	strncpy_s(f.website, m.website.c_str(), _TRUNCATE);
	strncpy_s(f.github, m.update_github.c_str(), _TRUNCATE);
	strncpy_s(f.target, m.target.c_str(), _TRUNCATE);
	strncpy_s(f.mode, m.mode.c_str(), _TRUNCATE);
	if (!m.provides_modes.empty())
	{
		strncpy_s(f.pmode_id, m.provides_modes[0].id.c_str(), _TRUNCATE);
		strncpy_s(f.pmode_title, m.provides_modes[0].title.c_str(), _TRUNCATE);
	}
	JoinList(m.requires_list, f.requires_, sizeof(f.requires_), "\n");
	JoinList(m.after, f.after, sizeof(f.after), "\n");
	JoinList(m.before, f.before, sizeof(f.before), "\n");
	JoinList(m.conflicts, f.conflicts, sizeof(f.conflicts), "\n");
	JoinList(m.exclude, f.exclude, sizeof(f.exclude), "\n");
	JoinList(m.tags, f.tags, sizeof(f.tags), ",");
	strncpy_s(f.pda, m.pda_layout.c_str(), _TRUNCATE);
}

// the form over a manifest read from disk: every other key of it stays as it is
static void ApplyForm(const SSettingsForm& f, EditorMod::SManifest& m)
{
	m.id = f.id;
	m.name = f.name;
	m.version = f.version;
	m.author = f.author;
	m.description = f.description;
	Trim(m.description);
	m.website = f.website;
	// a pasted link often drags a space or a line break along
	Trim(m.website);
	EditorMod::NormalizeGithubRepo(f.github, m.update_github);
	m.mode = f.mode;
	m.target = f.target;
	// only the first declared mode is edited here; an empty id drops it
	if (f.pmode_id[0])
	{
		if (m.provides_modes.empty()) m.provides_modes.push_back(EditorMod::SProvidesMode());
		m.provides_modes[0].id = f.pmode_id;
		m.provides_modes[0].title = f.pmode_title;
	}
	else if (!m.provides_modes.empty())
		m.provides_modes.erase(m.provides_modes.begin());
	SplitList(m.requires_list, f.requires_, '\n');
	SplitList(m.after, f.after, '\n');
	SplitList(m.before, f.before, '\n');
	SplitList(m.conflicts, f.conflicts, '\n');
	SplitList(m.exclude, f.exclude, '\n');
	SplitList(m.tags, f.tags, ',');
	m.pda_layout = f.pda;
}

static bool SettingsDirty()
{
	if (!s_ThumbPending.empty() || s_ThumbRemove) return true;
	for (const SFormField& field : kFormFields)
		if (strcmp(FormField(s_Form, field), FormField(s_FormBase, field)))
			return true;
	return false;
}

static void RefreshPdaFiles()
{
	s_PdaFiles.clear();
	xr_vector<xr_string> files;
	EditorPda::List(EditorProject::Root(), files);
	for (const xr_string& abs : files)
		s_PdaFiles.push_back(NqUtil::ProjectRelative(abs.c_str()));
	std::sort(s_PdaFiles.begin(), s_PdaFiles.end());
}

// the form as mod.ltx says it, edits dropped
static void LoadSettingsForm()
{
	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m))
	{
		EditorMod::EnsureScaffold(EditorProject::Root(), EditorProject::Name());
		EditorMod::Load(EditorProject::Root(), m);
	}
	FormFromManifest(m, s_FormBase);
	s_Form = s_FormBase;
	s_SettingsThumb = m.thumbnail;
	s_FormGen = EditorMod::ManifestGeneration();
	s_FormRoot = EditorProject::Root();
	s_SettingsStatus.clear();
	ThumbnailCleanup();

	string_path probe;
	sprintf_s(probe, "%s\\gamedata", EditorProject::Root());
	s_GatedFiles = CountFilesRec(probe, 0);
	sprintf_s(probe, "%s\\patch", EditorProject::Root());
	s_GatedPatches = CountFilesRec(probe, "_readme.txt");
	s_SlotChecked = false;
	s_InstalledScanned = false;
	RefreshPdaFiles();
}

// mod.ltx saved by somebody else: the fields the author has not touched follow it
static void SyncSettingsForm()
{
	if (s_FormRoot != EditorProject::Root())
	{
		LoadSettingsForm();
		return;
	}
	if (s_FormGen == EditorMod::ManifestGeneration()) return;
	s_FormGen = EditorMod::ManifestGeneration();
	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m)) return;
	SSettingsForm fresh;
	FormFromManifest(m, fresh);
	for (const SFormField& field : kFormFields)
		if (!strcmp(FormField(s_Form, field), FormField(s_FormBase, field)))
			strcpy_s(FormField(s_Form, field), field.size, FormField(fresh, field));
	s_FormBase = fresh;
	if (s_ThumbPending.empty() && !s_ThumbRemove)
		s_SettingsThumb = m.thumbnail;
	RefreshPdaFiles();
}

// 0 the plain game, 1 a mode of the linked game, 2 a mode the module brings, 3 every mode; -1 not chosen
static int SettingsKind()
{
	const SSettingsForm& f = s_Form;
	if (0 == _stricmp(f.target, "default"))		return 0;
	if (0 == _stricmp(f.target, "existing"))	return 1;
	if (0 == _stricmp(f.target, "new"))			return 2;
	if (0 == _stricmp(f.target, "any"))			return 3;
	// a manifest older than the target key: read off the mode fields
	if (0 == strcmp(f.mode, "*"))				return 3;
	if (f.pmode_id[0])							return 2;
	if (f.mode[0])								return 1;
	return -1;
}

// The module id of a relation line: a [requires] line may carry a value after '=' (other.mod = >=1.0), which the
// game does not read; the id is what it compares, in lower case.
static xr_string RelationId(const xr_string& line)
{
	xr_string id = line.substr(0, line.find('='));
	Trim(id);
	for (char& c : id) c = (char)tolower((u8)c);
	return id;
}

static void RelationIds(LPCSTR text, xr_vector<xr_string>& ids)
{
	xr_vector<xr_string> lines;
	SplitList(lines, text, '\n');
	ids.clear();
	for (const xr_string& line : lines)
		ids.push_back(RelationId(line));
}

static void CheckRelations(const SSettingsForm& f, xr_vector<SSettingsProblem>& out)
{
	struct SRel { LPCSTR field; LPCSTR text; };
	const SRel rels[] = { { "requires", f.requires_ }, { "after", f.after }, { "before", f.before }, { "conflicts", f.conflicts } };
	xr_vector<xr_string> ids[4];
	for (int i = 0; i < 4; ++i)
		RelationIds(rels[i].text, ids[i]);
	xr_string self = f.id;
	for (char& c : self) c = (char)tolower((u8)c);
	auto add = [&out](LPCSTR field, const xr_string& text)
	{
		SSettingsProblem& p = out.emplace_back();
		p.page = pgRelations;
		p.field = field;
		p.tone = XUI::Tone::Warning;
		p.blocks = false;
		p.text = text;
	};
	auto has = [](const xr_vector<xr_string>& v, const xr_string& id) { return std::find(v.begin(), v.end(), id) != v.end(); };
	for (int i = 0; i < 4; ++i)
		if (!self.empty() && has(ids[i], self))
			add(rels[i].field, "The module itself is listed here: the game ignores the line.");
	for (const xr_string& id : ids[3])
		if (has(ids[0], id))
			add("conflicts", "Required and in conflict at once: " + id + ". The game refuses this module with it and without it.");
	for (const xr_string& id : ids[2])
		if (has(ids[1], id))
			add("before", "Loads both before and after " + id + ": the game cannot keep both.");
}

static void CheckSettings()
{
	xr_vector<SSettingsProblem>& out = s_SettingsProblems;
	out.clear();
	const SSettingsForm& f = s_Form;
	auto add = [&out](int page, LPCSTR field, XUI::Tone tone, bool blocks, const xr_string& text)
	{
		SSettingsProblem& p = out.emplace_back();
		p.page = page;
		p.field = field;
		p.tone = tone;
		p.blocks = blocks;
		p.text = text;
	};

	if (!f.id[0])
		add(pgGeneral, "id", XUI::Tone::Error, true, "Required: the folder the module installs into.");
	else if (!EditorMod::ValidateId(f.id))
		add(pgGeneral, "id", XUI::Tone::Error, true, "Use a-z, 0-9, _ . and - only.");
	if (!EditorMod::ValidateVersion(f.version))
		add(pgGeneral, "version", XUI::Tone::Error, true, "1 to 4 numbers separated by dots, e.g. 1.2.0.");
	xr_string website = f.website;
	Trim(website);
	if (!EditorMod::ValidateWebsite(website.c_str()))
		add(pgGeneral, "website", XUI::Tone::Error, true, "Only https://ap-pro.ru and https://moddb.com links are allowed.");
	// the listing wants the module's own page, which the game's own check does not ask for
	else if (!website.empty() && !EditorCatalog::WebsiteIsModulePage(website.c_str()))
		add(pgGeneral, "website", XUI::Tone::Warning, false,
			"The Mod Browser lists a module with its own page, e.g. https://ap-pro.ru/stuff/<section>/<name>-r<number>/.");
	xr_string github;
	EditorMod::NormalizeGithubRepo(f.github, github);
	if (!EditorMod::ValidateGithubRepo(github.c_str()))
		add(pgGeneral, "github", XUI::Tone::Error, true, "owner/repo, e.g. Madmer/agroprom-story.");

	if (!s_ThumbError.empty())
		add(pgBrowser, "picture", XUI::Tone::Error, false, s_ThumbError);

	// the choice sticks while its details are still blank, so the missing detail is named instead
	const int kind = SettingsKind();
	if (kind < 0)
		add(pgMode, "target", XUI::Tone::Required, false, "Not chosen yet: the module cannot be built.");
	else if (kind == 1 && !f.mode[0])
		add(pgMode, "mode", XUI::Tone::Required, false, "Pick the mode: the module cannot be built until then.");
	else if (kind == 2 && !f.pmode_id[0])
		add(pgMode, "pmode_id", XUI::Tone::Required, false, "Enter the mode's id: the module cannot be built until then.");
	// "*" is the engine's no-gate wildcard: legal for mode=, never for a mode of the module's own
	if (strcmp(f.mode, "*") && !EditorMod::ValidateModeId(f.mode))
		add(pgMode, "mode", XUI::Tone::Error, true, "Use a-z, 0-9, _ . and - only.");
	if (!EditorMod::ValidateModeId(f.pmode_id))
		add(pgMode, "pmode_id", XUI::Tone::Error, true, "Use a-z, 0-9, _ . and - only.");
	// The caption ships in a cp1251 string table: anything outside that page reaches the
	// new-game screen as a look-alike or '?', while this field keeps showing the original.
	xr_string bad;
	if (kind == 2 && !NqUtil::Cp1251Covers(f.pmode_title, bad))
		add(pgMode, "pmode_title", XUI::Tone::Warning, false,
			"Outside cp1251 (" + bad + "): the new-game screen shows a look-alike letter or '?'.");

	// the file is looked at when the choice or mod.ltx changes, not every frame
	static xr_string s_PdaCheckedFor;
	static u32 s_PdaCheckedGen = u32(-1);
	static int s_PdaState = 0;
	if (s_PdaCheckedFor != f.pda || s_PdaCheckedGen != EditorMod::ManifestGeneration())
	{
		s_PdaCheckedFor = f.pda;
		s_PdaCheckedGen = EditorMod::ManifestGeneration();
		string_path abs;
		sprintf_s(abs, "%s\\%s", EditorProject::Root(), f.pda);
		s_PdaState = !f.pda[0] ? 0 : !FileExists(abs) ? 1 : EditorMod::OutsideBuild(EditorProject::Root(), f.pda) ? 2 : 0;
	}
	if (s_PdaState == 1)
		add(pgPda, "pda", XUI::Tone::Warning, false, "The file does not exist: the build refuses the module until it does.");
	else if (s_PdaState == 2)
		add(pgPda, "pda", XUI::Tone::Warning, false, "The build leaves this file out, so the game would never see it.");

	CheckRelations(f, out);
}

static int SettingsBlocking(const SSettingsProblem** first = nullptr)
{
	int n = 0;
	for (const SSettingsProblem& p : s_SettingsProblems)
		if (p.blocks)
		{
			if (!n && first) *first = &p;
			++n;
		}
	return n;
}

static int RowFlags(LPCSTR field, int flags = 0)
{
	for (const SSettingsProblem& p : s_SettingsProblems)
		if (!strcmp(p.field, field) && p.tone == XUI::Tone::Error)
			return flags | XUI::PropInvalid;
	return flags;
}

static void RowMessages(LPCSTR field)
{
	for (const SSettingsProblem& p : s_SettingsProblems)
		if (!strcmp(p.field, field))
			XUI::PropertyMessage(p.tone, p.text.c_str());
}

// the worst tone a page has: the dot beside it in the nav
static bool PageTone(int page, XUI::Tone& tone)
{
	int rank = 0;
	for (const SSettingsProblem& p : s_SettingsProblems)
	{
		if (p.page != page) continue;
		const int r = p.tone == XUI::Tone::Error ? 3 : p.tone == XUI::Tone::Required ? 2 : 1;
		if (r > rank)
		{
			rank = r;
			tone = p.tone;
		}
	}
	return rank > 0;
}

static bool SaveSettings(xr_string& err)
{
	CheckSettings();
	const SSettingsProblem* first = nullptr;
	if (SettingsBlocking(&first))
	{
		err = xr_string(kSettingsPages[first->page].label) + ": " + first->text;
		s_SettingsStatus = err;
		return false;
	}
	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m))
	{
		EditorMod::EnsureScaffold(EditorProject::Root(), EditorProject::Name());
		EditorMod::Load(EditorProject::Root(), m);
	}
	ApplyForm(s_Form, m);
	// the picture: the one waiting in temp\ takes its place, or the removal is carried out
	xr_string thumb_err;
	if (!s_ThumbPending.empty())
	{
		if (PlaceThumbnail(EditorProject::Root(), s_ThumbPending, thumb_err))
		{
			m.thumbnail = kThumbnailName;
			s_ThumbPending.clear();
		}
	}
	else if (s_ThumbRemove)
		EditorMod::ApplyThumbnail(EditorProject::Root(), "", m, thumb_err);
	if (!EditorMod::Save(EditorProject::Root(), m))
	{
		err = "mod.ltx cannot be written";
		s_SettingsStatus = err;
		return false;
	}
	s_ThumbRemove = false;
	s_ThumbError = thumb_err;
	// what was written, normalized (the repository as owner/repo, the description trimmed)
	FormFromManifest(m, s_FormBase);
	s_Form = s_FormBase;
	s_SettingsThumb = m.thumbnail;
	s_FormGen = EditorMod::ManifestGeneration();
	s_SettingsStatus = thumb_err.empty() ? xr_string() : "The picture was not saved: " + thumb_err;
	return true;
}

static void RevertSettings()
{
	s_Form = s_FormBase;
	if (!s_ThumbPending.empty()) ::DeleteFileA(s_ThumbPending.c_str());
	s_ThumbPending.clear();
	s_ThumbRemove = false;
	s_ThumbError.clear();
	s_SettingsStatus.clear();
}

// ---- the pages ------------------------------------------------------------------

static void DrawGeneralPage()
{
	SSettingsForm& f = s_Form;
	if (XUI::BeginPropertyGrid("mod_settings", .28f))
	{
		if (XUI::PropertyRow("Module ID", "Technical, never shown to the player: the folder the module installs into (<game>\\modules\\<id>) "
			"and the name other modules require it by. Seeded from the project folder - change it before the first release.",
			RowFlags("id", XUI::PropRequired)))
		{
			ImGui::InputText("##id", f.id, sizeof(f.id));
			RowMessages("id");
		}
		if (XUI::PropertyRow("Name", "The mod's title, as the game's Mods menu prints it. Free text: it owes nothing to the module id "
			"or the project folder, and it is not the title of a game mode."))
			ImGui::InputText("##name", f.name, sizeof(f.name));
		if (XUI::PropertyRow("Version", "1 to 4 numbers separated by dots. The game compares versions to offer an update.",
			RowFlags("version", XUI::PropRequired)))
		{
			ImGui::InputText("##version", f.version, sizeof(f.version));
			RowMessages("version");
		}
		if (XUI::PropertyRow("Author", "Shown in the game's Mods menu."))
			ImGui::InputText("##author", f.author, sizeof(f.author));
		XUI::EndPropertyGrid();
	}
	XUI::TextBlock("Description", "Shown in the game's Mods menu. Plain text or a string table id.", "##description",
		f.description, sizeof(f.description), 4);
	if (XUI::BeginPropertyGrid("mod_settings", .28f))
	{
		if (XUI::PropertyRow("Website", "The mod's page on AP-PRO or ModDB. The game opens it from the Mods menu.", RowFlags("website")))
		{
			ImGui::InputText("##website", f.website, sizeof(f.website));
			RowMessages("website");
		}
		if (XUI::PropertyRow("GitHub Repository", "owner/repo. The game checks the latest release of this repository for updates. "
			"A pasted address is fine: it is kept as owner/repo.", RowFlags("github")))
		{
			ImGui::InputText("##github", f.github, sizeof(f.github));
			if (ImGui::IsItemDeactivatedAfterEdit())
			{
				xr_string repo;
				EditorMod::NormalizeGithubRepo(f.github, repo);
				strncpy_s(f.github, repo.c_str(), _TRUNCATE);
			}
			RowMessages("github");
		}
		XUI::EndPropertyGrid();
	}
}

// The picture and what it becomes as one grid row: the frame, then its state and the buttons beside it. The grid keeps
// the height a row took, so a value this tall scrolls like any other.
static void PictureRow()
{
	if (!XUI::PropertyRow("Picture", "PNG, JPG, TGA or BMP. The middle square is scaled to 256 x 256 and saved as thumbnail.dds at the "
		"root of the module.", RowFlags("picture")))
		return;
	const xr_string shown = !s_ThumbPending.empty() ? s_ThumbPending :
		(!s_ThumbRemove && !s_SettingsThumb.empty()) ? xr_string(EditorProject::Root()) + "\\" + s_SettingsThumb : xr_string();
	if (shown != s_ThumbTextureOf)
	{
		// not drawn yet this frame, and the last frame is presented: safe to let go
		XFinedMCP::ReleaseTexture(s_ThumbTexture);
		s_ThumbTexture = shown.empty() ? 0 : DX11TextureFromFile(shown.c_str());
		s_ThumbTextureOf = shown;
	}

	const ImVec2 box(XUI::Pt(96.f), XUI::Pt(96.f));
	const ImVec2 at = ImGui::GetCursorScreenPos();
	ImDrawList* dl = ImGui::GetWindowDrawList();
	dl->AddRectFilled(at, ImVec2(at.x + box.x, at.y + box.y), XFinedTheme::U32(ModToken::Inset), XUI::Pt(4.f));
	if (s_ThumbTexture)
		ImGui::Image((ImTextureID)s_ThumbTexture, box);
	else
	{
		// the frame of the picture, so the row does not jump
		dl->AddRect(at, ImVec2(at.x + box.x, at.y + box.y), XFinedTheme::U32(ModToken::BorderSubtle), XUI::Pt(4.f));
		XUI::DrawIcon(dl, XUI::Icon::Picture, ImVec2(at.x + box.x * .5f - XUI::Pt(16.f), at.y + box.y * .5f - XUI::Pt(16.f)), 32.f,
			XFinedTheme::U32(ModToken::TextMuted));
		ImGui::Dummy(box);
	}
	ImGui::SameLine(0.f, XUI::Pt(12.f));
	ImGui::BeginGroup();
	const bool own = 0 == _stricmp(s_SettingsThumb.c_str(), kThumbnailName);
	if (!s_ThumbPending.empty())
		XUI::Prose(own || s_SettingsThumb.empty() ? "A new picture: it becomes thumbnail.dds when you save." : "A new picture: it is saved as thumbnail.dds.");
	else if (s_ThumbRemove)
		XUI::Prose(own ? "thumbnail.dds is deleted for good when you save." : "The manifest stops naming it when you save; the file stays.");
	else if (s_SettingsThumb.empty())
		XUI::Prose("No picture: the Mods window shows the first letter of the name.");
	else
	{
		XUI::TextEllipsis(s_SettingsThumb.c_str(), -1.f, XUI::Clip::Start);
		if (!s_ThumbTexture)
			XUI::StatusText(XUI::Tone::Error, "The file does not read as a picture.");
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
	if (XUI::Button("Choose Picture...", XUI::Kind::Secondary, !s_ThumbPickQueued, "The picture dialog is open"))
		s_ThumbPickQueued = UI->DeferUIWork(&PickThumbnail);
	const bool has_picture = !s_ThumbPending.empty() || (!s_ThumbRemove && !s_SettingsThumb.empty());
	if (has_picture)
	{
		ImGui::SameLine();
		if (XUI::Button("Remove", XUI::Kind::Ghost))
		{
			if (!s_ThumbPending.empty()) ::DeleteFileA(s_ThumbPending.c_str());
			s_ThumbPending.clear();
			s_ThumbRemove = !s_SettingsThumb.empty();
		}
	}
	else if (s_ThumbRemove)
	{
		ImGui::SameLine();
		if (XUI::Button("Keep Picture", XUI::Kind::Ghost))
			s_ThumbRemove = false;
	}
	ImGui::EndGroup();
	RowMessages("picture");
}

// The fixed vocabulary as checkboxes in the value column, as many columns as its width takes
static void TagsRow()
{
	if (!XUI::PropertyRow("Tags", "Players filter the Mod Browser by these."))
		return;
	const xr_vector<xr_string>& vocabulary = EditorCatalog::TagVocabulary();
	xr_vector<xr_string> tags;
	SplitList(tags, s_Form.tags, ',');
	const float width = ImGui::CalcItemWidth();
	const int columns = clampr(int(width / XUI::Em(9.f)), 1, 4);
	if (ImGui::BeginTable("##tags", columns, ImGuiTableFlags_SizingStretchSame, ImVec2(width, 0.f)))
	{
		for (u32 i = 0; i < vocabulary.size(); ++i)
		{
			ImGui::TableNextColumn();
			bool on = std::find(tags.begin(), tags.end(), vocabulary[i]) != tags.end();
			if (ImGui::Checkbox(EditorCatalog::TagLabel(vocabulary[i].c_str()), &on))
			{
				// kept in the vocabulary's order; a hand-written word the game ignores goes
				xr_vector<xr_string> kept;
				for (u32 k = 0; k < vocabulary.size(); ++k)
				{
					const bool had = std::find(tags.begin(), tags.end(), vocabulary[k]) != tags.end();
					if (k == i ? on : had) kept.push_back(vocabulary[k]);
				}
				JoinList(kept, s_Form.tags, sizeof(s_Form.tags), ",");
			}
		}
		ImGui::EndTable();
	}
}

static void DrawBrowserPage()
{
	if (!XUI::BeginPropertyGrid("mod_settings", .28f)) return;
	PictureRow();
	TagsRow();
	XUI::EndPropertyGrid();
}

static void SetSettingsKind(int kind, const xr_vector<EditorGameModes::SMode>& modes)
{
	SSettingsForm& f = s_Form;
	switch (kind)
	{
	case 0:
		f.mode[0] = f.pmode_id[0] = f.pmode_title[0] = 0;
		strcpy_s(f.target, "default");
		break;
	case 1:
		f.pmode_id[0] = f.pmode_title[0] = 0;
		strcpy_s(f.target, "existing");
		if ((!f.mode[0] || !strcmp(f.mode, "*")) && !modes.empty())
			strncpy_s(f.mode, modes[0].id.c_str(), _TRUNCATE);
		else if (!strcmp(f.mode, "*"))
			f.mode[0] = 0;
		break;
	case 2:
		f.mode[0] = 0;
		strcpy_s(f.target, "new");
		break;
	case 3:
		f.pmode_id[0] = f.pmode_title[0] = 0;
		strcpy_s(f.mode, "*");
		strcpy_s(f.target, "any");
		break;
	}
}

static void DrawModePage()
{
	SSettingsForm& f = s_Form;
	// The list is scanned from the linked game (a global mod like Revolution II replaces the stock one) and
	// shown by the names the player sees: nobody should have to know that Dead Air Metro is new_game_metro_mode.
	xr_string scan_err;
	EditorGameModes::Scan(scan_err);
	const xr_vector<EditorGameModes::SMode>& modes = EditorGameModes::All();
	const int kind = SettingsKind();
	static const char* kKinds[] = { "Plain Game", "Existing Mode", "New Mode", "Every Mode" };
	static const char* kAbout[] =
	{
		"Applies only while no campaign mode is active: Revolution II and the like do not see this module's level work.",
		"The level work applies only in that mode.",
		"A new mode is a new game of its own: the player picks it on the new-game screen, by its title.",
		"No gate: the level work lands in the plain game and in every campaign. Use it only for what belongs everywhere.",
	};
	if (!XUI::BeginPropertyGrid("mod_settings", .28f)) return;
	if (XUI::PropertyRow("Target", "Which game the module's level work applies in: spawns, collision, AI map, visuals and "
		"the game graph are gated on the mode.", RowFlags("target", XUI::PropRequired)))
	{
		int pick = kind;
		if (XUI::Segmented("##target", &pick, kKinds, 4))
			SetSettingsKind(pick, modes);
		if (kind >= 0)
			XUI::PropertyMessage(XUI::Tone::Muted, kAbout[kind]);
		RowMessages("target");
		// Files, configs and scripts mount at engine start, before any mode is picked: a weapon
		// rebalance would leak into every campaign, and the author should hear it here.
		if (kind > 0 && (s_GatedFiles || s_GatedPatches))
			XUI::PropertyMessage(XUI::Tone::Warning, (CountText(s_GatedFiles, "file", "files") + " under gamedata and " +
				CountText(s_GatedPatches, "config patch", "config patches") + " are not gated: they mount at engine start, in every game.").c_str());
	}
	if (kind == 1)
	{
		if (XUI::PropertyRow("Mode", "A mode of the linked game, by the name its new-game screen shows.", RowFlags("mode", XUI::PropRequired)))
		{
			xr_string current = f.mode;
			for (const EditorGameModes::SMode& mode : modes)
				if (0 == _stricmp(mode.id.c_str(), f.mode)) current = mode.caption;
			if (ImGui::BeginCombo("##mode", current.empty() ? "None" : current.c_str(), ImGuiComboFlags_HeightLarge))
			{
				for (u32 i = 0; i < modes.size(); ++i)
				{
					ImGui::PushID((int)i);
					if (ImGui::Selectable(modes[i].caption.c_str(), 0 == _stricmp(modes[i].id.c_str(), f.mode)))
						strncpy_s(f.mode, modes[i].id.c_str(), _TRUNCATE);
					XUI::Tip((modes[i].id + (modes[i].source == "game" ? xr_string(" - the linked game's") : " - module " + modes[i].source)).c_str());
					ImGui::PopID();
				}
				ImGui::EndCombo();
			}
			if (XUI::PropertyTrailing("##rescan", XUI::Icon::Refresh, "Scan the linked game's modes again"))
			{
				EditorGameModes::Invalidate();
				s_SlotChecked = false;
			}
			if (modes.empty())
				XUI::PropertyMessage(XUI::Tone::Warning, scan_err.empty() ? "The linked game declares no modes, only options." : scan_err.c_str());
			RowMessages("mode");
		}
	}
	else if (kind == 2)
	{
		if (XUI::PropertyRow("Mode ID", "Technical id of the MODE, not of the mod: the module's level work is gated on it and the "
			"new-game checkbox is built from it. It may match the module id or not.", RowFlags("pmode_id", XUI::PropRequired)))
		{
			ImGui::InputText("##pmode_id", f.pmode_id, sizeof(f.pmode_id));
			RowMessages("pmode_id");
		}
		if (XUI::PropertyRow("Mode Title", "The caption of the mode's checkbox on the new-game screen. It names the mode, not the mod: "
			"the Mods menu shows the Name on General instead."))
		{
			ImGui::InputText("##pmode_title", f.pmode_title, sizeof(f.pmode_title));
			RowMessages("pmode_title");
		}
		// A mode nobody can pick does not exist. The build generates the files that put it on the new-game
		// screen, and refuses the module when the linked game's screen cannot be read: that is measured once.
		if (!s_SlotChecked)
		{
			s_SlotChecked = true;
			EditorGameModes::SSlot slot;
			s_SlotError.clear();
			s_SlotOk = EditorGameModes::SuggestSlot("configs\\ui\\ui_mm_faction_select_16.xml", 1, f.id, slot, s_SlotError) ||
				EditorGameModes::SuggestSlot("configs\\ui\\ui_mm_faction_select.xml", 1, f.id, slot, s_SlotError);
		}
		if (XUI::PropertyRow("New-Game Screen", "Dead Air: Refined lists the mode among the module modes of the new-game screen; a screen "
			"without that list gets a checkbox instead. Every build writes the files for both and keeps them in step: "
			"gamedata\\configs\\ui\\<id>_modes.xml and <id>_modes_16.xml, gamedata\\configs\\text\\rus\\<id>_modes.xml (the caption) "
			"and scripts\\mode_register.script. Edit these settings, not those files. scripts\\register.script is yours and is never touched.",
			XUI::PropReadOnly))
		{
			XUI::PropertyText(s_SlotOk ? "Listed under the mode title" : "Not placed");
			if (!s_SlotOk)
				XUI::PropertyMessage(XUI::Tone::Warning, ("The build cannot read the linked game's new-game screen: " + s_SlotError + ".").c_str());
		}
	}
	XUI::EndPropertyGrid();
}

static const SInstalledModule* FindInstalled(const xr_string& id)
{
	for (const SInstalledModule& mod : s_Installed)
		if (mod.id == id) return &mod;
	return nullptr;
}

// Every module of the linked game: modules\, then the JSGME mods\ that still holds what was installed before the
// split - the same two roots the game reads.
static void ScanInstalledModules()
{
	s_InstalledScanned = true;
	s_Installed.clear();
	LPCSTR game = EditorProject::GameRoot();
	if (!game || !game[0]) return;
	static const char* kRoots[] = { "modules", "mods" };
	for (LPCSTR root : kRoots)
	{
		string_path mask;
		sprintf_s(mask, "%s\\%s\\*", game, root);
		WIN32_FIND_DATAA fd;
		HANDLE h = ::FindFirstFileA(mask, &fd);
		if (INVALID_HANDLE_VALUE == h) continue;
		do
		{
			// dot-names cover . and .. as well as tooling
			if (!(fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) || fd.cFileName[0] == '.') continue;
			string_path dir;
			sprintf_s(dir, "%s\\%s\\%s", game, root, fd.cFileName);
			EditorMod::SManifest m;
			if (!EditorMod::Load(dir, m)) continue;
			SInstalledModule mod;
			mod.id = RelationId(m.id.empty() ? xr_string(fd.cFileName) : m.id);
			mod.name = m.name;
			if (!mod.id.empty() && !FindInstalled(mod.id))
				s_Installed.push_back(mod);
		} while (::FindNextFileA(h, &fd));
		::FindClose(h);
	}
	std::sort(s_Installed.begin(), s_Installed.end(), [](const SInstalledModule& a, const SInstalledModule& b)
	{
		const xr_string& ka = a.name.empty() ? a.id : a.name;
		const xr_string& kb = b.name.empty() ? b.id : b.name;
		return _stricmp(ka.c_str(), kb.c_str()) < 0;
	});
}

// "Name  (id)" for a module the linked game has, the id alone otherwise
static xr_string ModuleLabel(const xr_string& id)
{
	const SInstalledModule* mod = FindInstalled(id);
	return mod && !mod->name.empty() && mod->name != id ? mod->name + "  (" + id + ")" : id;
}

// A module of the linked game, picked from a searchable list; the ones listed already and this module are left
// out. An id typed into the search is taken as it is, for a module this computer does not have.
static bool ModulePicker(LPCSTR popup, const xr_vector<xr_string>& taken, xr_string& picked)
{
	if (!ImGui::BeginPopup(popup)) return false;
	bool done = false;
	const float width = XUI::Em(24.f);
	if (ImGui::IsWindowAppearing())
	{
		s_ModuleSearch[0] = 0;
		ImGui::SetKeyboardFocusHere();
	}
	ImGui::SetNextItemWidth(width);
	XUI::SearchField("##search", s_ModuleSearch, sizeof(s_ModuleSearch), "Search, or type a module id");
	const xr_string self = RelationId(s_Form.id);
	const xr_string typed = RelationId(s_ModuleSearch);
	auto is_taken = [&](const xr_string& id) { return id == self || std::find(taken.begin(), taken.end(), id) != taken.end(); };
	// id, label: the matches, then the typed id when the list does not hold it
	xr_vector<std::pair<xr_string, xr_string>> rows;
	for (const SInstalledModule& mod : s_Installed)
	{
		const xr_string label = ModuleLabel(mod.id);
		if (!is_taken(mod.id) && XUI::Match(s_ModuleSearch, label.c_str()))
			rows.emplace_back(mod.id, label);
	}
	const bool typed_row = !typed.empty() && !FindInstalled(typed) && !is_taken(typed) && EditorMod::ValidateId(typed.c_str());
	if (typed_row)
		rows.emplace_back(typed, "Add " + typed);
	// Enter takes the first row
	if (!rows.empty() && ImGui::IsKeyPressed(ImGuiKey_Enter, false) && ImGui::IsWindowFocused(ImGuiFocusedFlags_ChildWindows))
	{
		picked = rows[0].first;
		done = true;
		ImGui::CloseCurrentPopup();
	}
	ImGui::BeginChild("##modules", ImVec2(width, XUI::Pt(240.f)));
	for (u32 i = 0; i < rows.size() && !done; ++i)
	{
		ImGui::PushID(rows[i].first.c_str());
		if (ImGui::Selectable(rows[i].second.c_str()))
		{
			picked = rows[i].first;
			done = true;
			ImGui::CloseCurrentPopup();
		}
		if (typed_row && i + 1 == rows.size())
			XUI::Tip("Not installed in the linked game: the id is taken as typed.");
		ImGui::PopID();
	}
	if (rows.empty())
	{
		LPCSTR game = EditorProject::GameRoot();
		XUI::StatusText(XUI::Tone::Muted, !s_Installed.empty() ? "No module matches. A module id typed here can be added as it is." :
			game && game[0] ? "The linked game has no other module. Type a module id to add it as it is." :
			"No game is linked. Type a module id to add it as it is.");
	}
	ImGui::EndChild();
	ImGui::EndPopup();
	return done;
}

// One relation as a grid row: its modules as chips, then Add Module..., the notes under them. The form keeps one line
// per module, so Save, Revert and the unsaved mark work as before, and a line written by hand (a value after '=') is
// kept as written.
static void RelationRow(LPCSTR field, LPCSTR label, LPCSTR help, char* buf, size_t n)
{
	if (!XUI::PropertyRow(label, help, RowFlags(field)))
		return;
	ImGui::PushID(field);
	// a group keeps the chips' second line and the button in the value column
	ImGui::BeginGroup();
	xr_vector<xr_string> lines, ids;
	SplitList(lines, buf, '\n');
	for (const xr_string& line : lines)
		ids.push_back(RelationId(line));
	bool changed = false;
	if (!lines.empty())
	{
		XUI::Strings chips;
		for (u32 i = 0; i < lines.size(); ++i)
		{
			const size_t eq = lines[i].find('=');
			xr_string value = eq == xr_string::npos ? xr_string() : lines[i].substr(eq + 1);
			Trim(value);
			chips.push_back(value.empty() ? ModuleLabel(ids[i]) : ModuleLabel(ids[i]) + "  " + value);
		}
		const int removed = XUI::Chips("##chips", chips, true);
		if (removed >= 0)
		{
			lines.erase(lines.begin() + removed);
			ids.erase(ids.begin() + removed);
			changed = true;
		}
	}
	if (XUI::Button("Add Module..."))
		XUI::Shell::OpenPopupAt("pick_module", ImVec2(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y + XUI::Pt(2.f)));
	xr_string picked;
	if (ModulePicker("pick_module", ids, picked))
	{
		ids.push_back(picked);
		lines.push_back(picked);
		changed = true;
	}
	ImGui::EndGroup();
	if (changed)
		JoinList(lines, buf, (u32)n, "\n");
	// modules this computer does not have: kept, said
	LPCSTR game = EditorProject::GameRoot();
	if (game && game[0])
	{
		xr_string missing;
		for (const xr_string& id : ids)
			if (!FindInstalled(id))
				missing += (missing.empty() ? "" : ", ") + id;
		if (!missing.empty())
			XUI::PropertyMessage(XUI::Tone::Muted, ("Not installed in the linked game: " + missing + ".").c_str());
	}
	RowMessages(field);
	ImGui::PopID();
}

static void DrawRelationsPage()
{
	SSettingsForm& f = s_Form;
	if (!s_InstalledScanned)
		ScanInstalledModules();
	if (!XUI::BeginPropertyGrid("mod_settings", .28f)) return;
	RelationRow("requires", "Requires", "The game refuses to load this module without these.", f.requires_, sizeof(f.requires_));
	RelationRow("after", "Load After", "These load first; this module's changes go on top of theirs.", f.after, sizeof(f.after));
	RelationRow("before", "Load Before", "This module loads first; their changes go on top of its.", f.before, sizeof(f.before));
	RelationRow("conflicts", "Conflicts", "The game refuses to load this module together with these.", f.conflicts, sizeof(f.conflicts));
	XUI::EndPropertyGrid();
}

static void DrawContentsPage()
{
	XUI::TextBlock("Not Shipped", "Project paths or patterns, one per line:\n"
		"docs - a folder, with everything in it\n"
		"design/story.md - one file\n"
		"*.psd - every file of that name, at any depth\n"
		"docs/*.md - a pattern below one folder\n"
		"The Content Browser sets them too: right-click > Exclude from Build.",
		"##exclude", s_Form.exclude, sizeof(s_Form.exclude), 8);
	if (XUI::BeginPropertyGrid("mod_settings", .28f))
	{
		if (XUI::PropertyRow("Never Shipped", "Version control and tool files (.git, .gitignore, Thumbs.db, ...), the editor's own folders "
			"(releases, temp, backup, ...) and the sources under rawdata. No rule is needed for these.", XUI::PropReadOnly))
			XUI::PropertyText("Tooling, the editor's own folders, rawdata");
		XUI::EndPropertyGrid();
	}
}

static void DrawPdaPage()
{
	SSettingsForm& f = s_Form;
	if (!XUI::BeginPropertyGrid("mod_settings", .28f)) return;
	if (XUI::PropertyRow("Layout", "The .pdaasset of the project the game's PDA shows. None: the module leaves the PDA alone.", RowFlags("pda")))
	{
		if (ImGui::BeginCombo("##pda", f.pda[0] ? f.pda : "None", ImGuiComboFlags_HeightLarge))
		{
			if (ImGui::Selectable("None", !f.pda[0]))
				f.pda[0] = 0;
			for (const xr_string& rel : s_PdaFiles)
				if (ImGui::Selectable(rel.c_str(), EditorPda::SamePath(rel.c_str(), f.pda)))
					strncpy_s(f.pda, rel.c_str(), _TRUNCATE);
			ImGui::EndCombo();
		}
		if (f.pda[0] && XUI::PropertyTrailing("##open_pda", XUI::Icon::External, "Open it in the PDA Interface tab"))
		{
			string_path abs;
			sprintf_s(abs, "%s\\%s", EditorProject::Root(), f.pda);
			EditorPda::Open(abs);
		}
		if (s_PdaFiles.empty())
			XUI::PropertyMessage(XUI::Tone::Muted, "The project has no PDA interface yet.");
		RowMessages("pda");
	}
	XUI::EndPropertyGrid();
	if (s_PdaFiles.empty())
	{
		ImGui::SetCursorPosX(ImGui::GetCursorPosX() + XUI::Pt(6.f));
		// makes one from the template and names it in mod.ltx at once: the form follows that save
		if (XUI::Button("Create PDA Interface"))
			EditorPda::ShowModuleLayout();
	}
}

// ---- the tab ----------------------------------------------------------------------

static void DrawSettingsBody()
{
	if (!EditorProject::Active())
	{
		XUI::EmptyState(XUI::Icon::Settings, "No project", "Mod Settings edit the mod.ltx of the open project.");
		return;
	}
	SyncSettingsForm();
	CheckSettings();

	const float footer = XUI::Pt(48.f);
	const float nav_pt = _min(200.f, ImGui::GetContentRegionAvail().x / XUI::Dpi() * .3f);

	// the pages
	if (XUI::BeginList("##settings_nav", ImVec2(nav_pt, -48.f)))
	{
		for (int i = 0; i < pgCount; ++i)
		{
			XUI::NavRowSpec row;
			row.icon = kSettingsPages[i].icon;
			row.dot = PageTone(i, row.tone);
			if (XUI::NavRow(kSettingsPages[i].id, kSettingsPages[i].label, s_SettingsPage == i, row))
			{
				s_SettingsPage = i;
				if (i == pgPda) RefreshPdaFiles();
				// a module installed while the tab was open is on the list the next time the page is picked
				if (i == pgRelations) s_InstalledScanned = false;
			}
		}
	}
	XUI::EndList();
	ImGui::SameLine(0.f, XUI::Pt(8.f));

	// the form: one column of up to 44 em beside the nav, the scrollbar at the right edge of the tab
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(16.f), XUI::Pt(8.f)));
	// layout children: the document's own fill, not the inset of a list
	ImGui::PushStyleColor(ImGuiCol_ChildBg, XFinedTheme::U32(ModToken::Base));
	ImGui::BeginChild("##settings_form", ImVec2(0.f, -footer), ImGuiChildFlags_AlwaysUseWindowPadding);
	ImGui::PopStyleVar();
	{
		const float column = _min(ImGui::GetContentRegionAvail().x, XUI::Em(44.f));
		ImGui::BeginChild("##settings_column", ImVec2(column, 0.f), ImGuiChildFlags_AutoResizeY);
		ImGui::PopStyleColor();
		const SSettingsPage& page = kSettingsPages[s_SettingsPage];
		{
			XUI::FontScope title(XUI::Font::Title);
			XUI::TextEllipsis(page.label);
		}
		XUI::Prose(page.about);
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));
		switch (s_SettingsPage)
		{
		case pgGeneral:		DrawGeneralPage();	break;
		case pgBrowser:		DrawBrowserPage();	break;
		case pgMode:		DrawModePage();		break;
		case pgRelations:	DrawRelationsPage();	break;
		case pgContents:	DrawContentsPage();	break;
		case pgPda:			DrawPdaPage();		break;
		}
		ImGui::EndChild();
	}
	ImGui::EndChild();

	// the sticky footer: what stops the save on the left, Revert and Save on the right
	const ImVec2 at = ImGui::GetCursorScreenPos();
	const float right = at.x + ImGui::GetContentRegionAvail().x;
	ImGui::GetWindowDrawList()->AddLine(ImVec2(at.x, at.y), ImVec2(right, at.y),
		XFinedTheme::U32(ModToken::BorderSubtle));
	ImGui::SetCursorScreenPos(ImVec2(at.x, at.y + XUI::Pt(4.f)));
	const bool dirty = SettingsDirty();
	const SSettingsProblem* first = nullptr;
	const int blocking = SettingsBlocking(&first);
	xr_string why;
	if (blocking)
		why = xr_string(kSettingsPages[first->page].label) + ": " + first->text;
	{
		XUI::Toolbar tb("mod_settings_footer", XUI::ToolbarRow);
		tb.Group(1);
		if (blocking)
			tb.Status(blocking == 1 ? "1 problem to fix before saving" : NqUtil::Format("%d problems to fix before saving", blocking).c_str(), XUI::Tone::Error);
		else if (!s_SettingsStatus.empty())
			tb.Status(s_SettingsStatus.c_str(), XUI::Tone::Error);
		else
			tb.Status(dirty ? "Unsaved changes" : "mod.ltx is up to date", XUI::Tone::Muted);
		tb.Spring();
		tb.Group(3);
		if (tb.Action("revert", XUI::Icon::Undo, "Revert", dirty ? "Drop the changes: back to what mod.ltx says" : "Nothing to revert", dirty))
			RevertSettings();
		if (tb.Action("save", XUI::Icon::Save, "Save", !dirty ? "No changes to save" : blocking ? why.c_str() : "Write mod.ltx",
			dirty && !blocking, false, XUI::Kind::Primary))
		{
			xr_string err;
			SaveSettings(err);
		}
	}
}

static void ShowSettingsTab(bool activate)
{
	// the session reopens the tab with the project; registered on first use as well as at startup
	static bool s_Factory = false;
	if (!s_Factory)
	{
		s_Factory = true;
		XUI::Shell::RegisterFactory("mod_settings", [](LPCSTR) { EditorMod::RequestEditManifest(); return true; });
	}
	if (XUI::Shell::IsOpen("mod_settings"))
	{
		if (activate) XUI::Shell::Activate("mod_settings");
		return;
	}
	XUI::Shell::DocDesc d;
	d.id = "mod_settings";
	d.kind = "mod_settings";
	d.icon = XUI::Icon::Settings;
	d.label = [] { return xr_string("Mod Settings"); };
	d.tooltip = [] { return xr_string("mod.ltx: the module's name, version, game mode and what the build ships"); };
	d.alive = [] { return s_SettingsOpen; };
	d.dirty = [] { return SettingsDirty(); };
	d.body = [] { DrawSettingsBody(); };
	d.save = [](xr_string& err) { return SaveSettings(err); };
	d.discard = [] { RevertSettings(); };
	// the frame that showed the preview is presented by now: the texture may go
	d.on_close = [] { s_SettingsOpen = false; ThumbnailCleanup(); };
	XUI::Shell::Open(d, activate);
}

void EditorMod::RequestEditManifest()
{
	if (!EditorProject::Active()) return;
	if (!s_SettingsOpen || s_FormRoot != EditorProject::Root())
		LoadSettingsForm();
	s_SettingsOpen = true;
	ShowSettingsTab(true);
}

//------------------------------------------------------------------------------
// builds: Build Into Game, Build to Folder, Export Flat Gamedata
//
// Every question the export dialog asks has one right answer once a game is
// linked: the target is that game, the layout is a module, and the id comes
// from the manifest. So Build Into Game asks nothing - except before deleting
// files it did not put there.
//------------------------------------------------------------------------------
static bool					s_WantExport		= false;
static bool					s_WantFlat			= false;
static string_path			s_Target			= {};
static xr_string			s_ExportId;			// what the dialog names the destination by
static bool					s_WantBuild			= false;
static bool					s_BuildQueued		= false;
static bool					s_BuildClean		= false;	// the author agreed to the files going
static bool					s_ExportQueued		= false;
static bool					s_DeferredExportFlat = false;
static bool					s_DeferredClean		= false;
static string_path			s_DeferredExportTarget = {};
static EditorMod::TPreBuildBake	s_PreBuildBake	= 0;

void EditorMod::SetPreBuildBake(TPreBuildBake fn)	{ s_PreBuildBake = fn; }
void EditorMod::RunPreBuildBake(xr_string& note)	{ note.clear(); if (s_PreBuildBake) s_PreBuildBake(note); }

bool EditorMod::CanBuildIntoGame()
{
	return EditorProject::Active() && EditorProject::GameLinked() && EditorProject::GameRoot()[0];
}

void EditorMod::BuildTargetText(char* dst, u32 size)
{
	dst[0] = 0;
	if (!CanBuildIntoGame()) return;
	SManifest m;
	if (!Load(EditorProject::Root(), m) || !ValidateId(m.id.c_str())) return;
	sprintf_s(dst, size, "%s\\modules\\%s", EditorProject::GameRoot(), m.id.c_str());
}

static void ProjectIni(char* dst, u32 size)
{
	sprintf_s(dst, size, "%s\\project.ltx", EditorProject::Root());
}

static void ReportMissingId(LPCSTR title)
{
	XUI::Result("build_result", title, XUI::Tone::Error,
		"The module id is missing or invalid. Set it in Mod Settings: a-z, 0-9, _ . and - only.",
		xr_vector<std::pair<xr_string, xr_string>>(), XUI::Strings(), XUI::Strings(),
		"Mod Settings", [] { EditorMod::RequestEditManifest(); });
}

// A bake note is a line per layer and a WARNING line per thing left out: the first go to the
// rows of the result, the others to its warnings.
static void AddBakeNote(const xr_string& note, xr_vector<std::pair<xr_string, xr_string>>& rows, XUI::Strings& warnings)
{
	XUI::Strings lines;
	SplitLines(note, lines);
	for (const xr_string& line : lines)
	{
		if (!line.compare(0, 9, "WARNING: "))	warnings.push_back(line.substr(9));
		else									rows.emplace_back("Scene", line);
	}
}

// what a build or an export did, in the result dialog every build shares (UI runs only)
static void ReportBuild(bool into_game, bool flat, bool ok, int files, const xr_string& path, const xr_string& err,
						const xr_string& warn, const xr_vector<xr_string>& left_out, const xr_string& bake_note, const xr_string& id)
{
	xr_vector<std::pair<xr_string, xr_string>> rows;
	XUI::Strings warnings;
	if (ok) rows.emplace_back("Folder", path);
	AddBakeNote(bake_note, rows, warnings);
	SplitLines(warn, warnings);
	const char* result_id = flat ? "export_result" : "build_result";
	if (!ok)
	{
		XUI::Result(result_id, flat ? "Export Failed" : "Build Failed", XUI::Tone::Error, err.c_str(), rows, XUI::Strings(), warnings);
		return;
	}
	xr_string headline;
	if (into_game)
	{
		headline = "The module is in the game: " + CountText(files, "file", "files") + ". Added objects reach new games and loaded "
			"saves; changes to existing objects need a new game.";
		rows.emplace_back("Console", "xms_enable " + id + "  /  xms_disable " + id);
	}
	else if (flat)
		headline = CountText(files, "file", "files") + " exported as a gamedata folder.";
	else
		headline = "The module is built: " + CountText(files, "file", "files") + ".";
	XUI::Result(result_id, flat ? "Export Complete" : "Build Complete", warnings.empty() ? XUI::Tone::Success : XUI::Tone::Warning,
		headline.c_str(), rows, left_out, warnings, "Open Folder", [path] { OpenFolder(path); });
}

static bool IsCleanRefusal(const xr_string& err)
{
	return 0 == strncmp(err.c_str(), "target already holds a module", 29);
}

static void RunDeferredExport();

// The build is clean and refused to empty a folder of files it did not write: the second phase
// names them and, on Yes, runs the build again with `clean`.
static void AskClean(bool into_game, const xr_string& folder)
{
	const xr_string headline = folder + " holds " + CountText(s_StaleCount, "file", "files") + " this build does not write.";
	XUI::ConfirmSpec spec;
	spec.id = "clean_target";
	spec.title = "Replace Module Files";
	spec.headline = headline.c_str();
	spec.items = s_StaleNames;
	spec.total = (u32)s_StaleCount;
	spec.consequence = "Deleted for good - this cannot be undone. The folder then holds exactly this build.";
	spec.action = "Delete and Build";
	spec.destructive = true;
	XUI::Confirm(spec, [into_game](XUI::Answer answer)
	{
		if (answer != XUI::Answer::Yes)
		{
			XUI::Toast(XUI::Tone::Info, "Build cancelled: nothing was deleted.");
			return;
		}
		if (into_game)
		{
			s_WantBuild = true;
			s_BuildClean = true;
		}
		else
		{
			s_DeferredClean = true;
			s_ExportQueued = UI->DeferUIWork(&RunDeferredExport);
		}
	});
}

void EditorMod::RequestBuildIntoGame()
{
	s_WantBuild = CanBuildIntoGame();
	s_BuildClean = false;
	if (s_WantBuild) return;
	XUI::Result("build_result", "Build Failed", XUI::Tone::Error,
		"No game is linked. The build writes the module into <game>\\modules\\<id>, where the game reads it: link a game first.",
		xr_vector<std::pair<xr_string, xr_string>>(), XUI::Strings(), XUI::Strings());
}

static void RunBuildIntoGame()
{
	s_BuildQueued = false;
	if (!s_WantBuild) return;
	s_WantBuild = false;
	const bool clean = s_BuildClean;
	s_BuildClean = false;
	SProgressOperation progress(*UI, false);

	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m) || !EditorMod::ValidateId(m.id.c_str()))
	{
		ReportMissingId("Build Failed");
		return;
	}

	LPCSTR target = EditorProject::GameRoot();
	xr_string bake_note;
	EditorMod::RunPreBuildBake(bake_note);
	int files = 0;
	xr_string path, err, warn;
	xr_vector<xr_string> left_out;
	const bool ok = EditorMod::Export(EditorProject::Root(), target, false, files, path, err, clean, &warn, &left_out);
	if (!ok && IsCleanRefusal(err))
	{
		AskClean(true, path);
		return;
	}
	if (ok)
	{
		char ini[MAX_PATH];
		ProjectIni(ini, sizeof(ini));
		::WritePrivateProfileStringA("xms", "export_target", target, ini);
	}
	ReportBuild(true, false, ok, files, path, err, warn, left_out, bake_note, m.id);
}

static void PrepareExport(bool flat)
{
	if (!EditorProject::Active()) return;
	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m) || !EditorMod::ValidateId(m.id.c_str()))
	{
		ReportMissingId(flat ? "Export Flat Gamedata" : "Build to Folder");
		return;
	}
	s_ExportId = m.id;
	char ini[MAX_PATH];
	ProjectIni(ini, sizeof(ini));
	::GetPrivateProfileStringA("xms", "export_target", "", s_Target, sizeof(s_Target), ini);
	// first export of a linked project: the game root IS the deploy target -
	// the module lands in <game>\modules\<id>, which is where the game reads it
	if (!s_Target[0] && EditorProject::GameRoot()[0])
		strncpy_s(s_Target, sizeof(s_Target), EditorProject::GameRoot(), _TRUNCATE);
	if (flat)	s_WantFlat = true;
	else		s_WantExport = true;
}

void EditorMod::RequestExportModule()	{ PrepareExport(false); }
void EditorMod::RequestExportFlat()		{ PrepareExport(true); }

static void RunDeferredExport()
{
	s_ExportQueued = false;
	const bool clean = s_DeferredClean;
	s_DeferredClean = false;
	SProgressOperation progress(*UI, false);
	xr_string bake_note;
	EditorMod::RunPreBuildBake(bake_note);
	int files = 0;
	xr_string path, err, warn;
	xr_vector<xr_string> left_out;
	const bool ok = EditorMod::Export(EditorProject::Root(), s_DeferredExportTarget,
		s_DeferredExportFlat, files, path, err, clean, &warn, &left_out);
	if (!ok && IsCleanRefusal(err))
	{
		AskClean(false, path);
		return;
	}
	if (ok)
	{
		char ini[MAX_PATH];
		ProjectIni(ini, sizeof(ini));
		::WritePrivateProfileStringA("xms", "export_target", s_DeferredExportTarget, ini);
	}
	ReportBuild(false, s_DeferredExportFlat, ok, files, path, err, warn, left_out, bake_note, s_ExportId);
}

// A read-only path as a grid row's value. PropertyText cuts at the end, and the end of a destination (<folder>\modules\<id>)
// is what such a row is for: this keeps it and drops the start, the whole path in the tooltip. Selectable like
// PropertyText - at once when it fits, after a click when it is cut.
static void PropertyPath(const char* path)
{
	const ImGuiStyle& style = ImGui::GetStyle();
	const float width = ImGui::CalcItemWidth();
	const float room = width - style.FramePadding.x * 2.f;
	if (XUI::TextWidth(XUI::Font::Body, path) <= room)
	{
		XUI::PropertyText(path);
		return;
	}
	ImGuiStorage* storage = ImGui::GetStateStorage();
	const ImGuiID key = ImGui::GetID("##property_path");
	// 0: shown cut, 1: a read-only field to select from, 2: the same, taking the focus this frame
	const int state = storage->GetInt(key, 0);
	if (state)
	{
		xr_string copy = path;
		ImGui::PushStyleColor(ImGuiCol_FrameBg, IM_COL32(0, 0, 0, 0));
		ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::U32(ModToken::TextSecondary));
		ImGui::PushStyleVar(ImGuiStyleVar_FrameBorderSize, 0.f);
		ImGui::SetNextItemWidth(width);
		if (state == 2)
		{
			storage->SetInt(key, 1);
			ImGui::SetKeyboardFocusHere();
		}
		ImGui::InputText("##property_path", &copy[0], copy.size() + 1, ImGuiInputTextFlags_ReadOnly);
		if (ImGui::IsItemDeactivated())
			storage->SetInt(key, 0);
		ImGui::PopStyleVar();
		ImGui::PopStyleColor(2);
		return;
	}
	const ImVec2 at = ImGui::GetCursorScreenPos();
	ImGui::SetCursorScreenPos(ImVec2(at.x + style.FramePadding.x, at.y));
	ImGui::AlignTextToFramePadding();
	ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::U32(ModToken::TextSecondary));
	XUI::TextEllipsis(path, room, XUI::Clip::Start);
	ImGui::PopStyleColor();
	if (ImGui::IsItemClicked())
		storage->SetInt(key, 2);
}

static void DrawExportDialog(bool flat)
{
	bool& want = flat ? s_WantFlat : s_WantExport;
	if (!want) return;
	if (!XUI::BeginDialog(flat ? kTitleFlat : kTitleExport, &want, XUI::DialogSize::M))
		return;
	XUI::BeginDialogBody();
	XUI::Prose(flat
		? "A gamedata_<id> folder is made inside the folder: a JSGME mod, for installs without the module system. "
		  "Quests, spawns and the game mode need the module build."
		: "The module goes to <folder>\\modules\\<id>, where the game reads modules from. The build is clean: "
		  "that folder then holds exactly this build.");
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));
	if (XUI::BeginPropertyGrid("build_dialog", .30f))
	{
		if (XUI::PropertyRow(flat ? "Folder" : "Game Folder", flat ? "The folder the gamedata_<id> folder is made in." :
			"The game folder, or any folder that stands in for one.", XUI::PropRequired))
		{
			XUI::PathField("##target", s_Target, sizeof(s_Target), true);
			if (EditorProject::GameRoot()[0] &&
				XUI::PropertyTrailing("##linked", XUI::Icon::GameFolder, (xr_string("Use the linked game: ") + EditorProject::GameRoot()).c_str()))
				strncpy_s(s_Target, sizeof(s_Target), EditorProject::GameRoot(), _TRUNCATE);
		}
		if (s_Target[0] && XUI::PropertyRow("Writes To", "The folder this build fills.", XUI::PropReadOnly))
		{
			xr_string dest = s_Target;
			while (!dest.empty() && (dest.back() == '\\' || dest.back() == '/')) dest.pop_back();
			dest += flat ? "\\gamedata_" + s_ExportId : "\\modules\\" + s_ExportId;
			PropertyPath(dest.c_str());
		}
		XUI::EndPropertyGrid();
	}
	if (XUI::BeginDialogFooter())
	{
		if (XUI::DialogButton(flat ? "Export" : "Build", XUI::Kind::Primary, true, s_Target[0] && !s_ExportQueued,
			s_ExportQueued ? "A build is running" : "Pick a folder first"))
		{
			strncpy_s(s_DeferredExportTarget, sizeof(s_DeferredExportTarget), s_Target, _TRUNCATE);
			s_DeferredExportFlat = flat;
			s_DeferredClean = false;
			s_ExportQueued = UI->DeferUIWork(&RunDeferredExport);
			want = false;
		}
		if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
			want = false;
	}
	XUI::EndDialog();
}

//------------------------------------------------------------------------------
// Mod > Package Release...
//------------------------------------------------------------------------------
static bool					s_WantPackage		= false;
static bool					s_PackageQueued		= false;
static EditorMod::SManifest	s_PkgManifest;		// what the dialog names: id, version, update source
static string_path			s_PkgOutDir			= {};
static char					s_PkgRequires[32]	= {};

void EditorMod::RequestPackageRelease()
{
	// a queued packaging still reads the fields below
	if (!EditorProject::Active() || s_PackageQueued) return;
	xr_string err;
	if (!Load(EditorProject::Root(), s_PkgManifest))
		err = "mod.ltx not found in the project";
	else
		ReleaseGate(s_PkgManifest, err);
	if (!err.empty())
	{
		XUI::Result("package_result", "Cannot Package", XUI::Tone::Error, (err + ".").c_str(),
			xr_vector<std::pair<xr_string, xr_string>>(), XUI::Strings(), XUI::Strings(),
			"Mod Settings", [] { EditorMod::RequestEditManifest(); });
		return;
	}
	sprintf_s(s_PkgOutDir, "%s\\releases\\%s", EditorProject::Root(), s_PkgManifest.version.c_str());
	xr_string game_version;
	LinkedGameVersion(game_version);
	strncpy_s(s_PkgRequires, sizeof(s_PkgRequires), game_version.c_str(), _TRUNCATE);
	s_WantPackage = true;
}

static void RunDeferredPackage()
{
	s_PackageQueued = false;
	SProgressOperation progress(*UI, true);
	xr_string bake_note;
	EditorMod::RunPreBuildBake(bake_note);
	EditorMod::SPackageResult res;
	xr_string err, warn;
	const EditorMod::SManifest& m = s_PkgManifest;
	xr_vector<std::pair<xr_string, xr_string>> rows;
	XUI::Strings warnings;
	if (!EditorMod::PackageRelease(EditorProject::Root(), s_PkgOutDir, s_PkgRequires, res, err, &warn))
	{
		if (err == "cancelled")
		{
			XUI::Toast(XUI::Tone::Info, "Packaging cancelled.");
			return;
		}
		AddBakeNote(bake_note, rows, warnings);
		XUI::Result("package_result", "Packaging Failed", XUI::Tone::Error, err.c_str(), rows, XUI::Strings(), warnings);
		return;
	}

	rows.emplace_back("Folder", res.out_dir);
	rows.emplace_back("Descriptor", m.id + ".update.ltx");
	rows.emplace_back("File index", res.index.name);
	for (const EditorMod::SPackageFile& p : res.packages)
		rows.emplace_back("Package", NqUtil::Format("%s  (%.1f MB)", p.name.c_str(), double(p.size) / 1048576.0));
	rows.emplace_back("Signed", res.signed_ ? "With the author key " + res.key_id : xr_string("No - the Mod Browser lists signed releases only"));
	AddBakeNote(bake_note, rows, warnings);
	if (m.update_github.empty())
		warnings.push_back("No GitHub repository in Mod Settings: the game does not look for updates of this module.");
	SplitLines(warn, warnings);
	// the index addresses the zip by byte offset: a repacked copy is another file
	const xr_string headline = NqUtil::Format("Release %s %s: %s packaged. Upload the files exactly as written to a GitHub release%s "
		"and mark it as the latest - Publish Release does both in one step.", m.id.c_str(), m.version.c_str(),
		CountText(res.files, "file", "files").c_str(), m.update_github.empty() ? "" : (" of " + m.update_github).c_str());
	XUI::Result("package_result", "Release Packaged", warnings.empty() ? XUI::Tone::Success : XUI::Tone::Warning, headline.c_str(),
		rows, res.left_out, warnings, "Open Folder", [dir = res.out_dir] { OpenFolder(dir); });
}

static void QueuePackage()
{
	s_PackageQueued = UI->DeferUIWork(&RunDeferredPackage);
	s_WantPackage = false;
}

static void DrawPackageDialog()
{
	if (!s_WantPackage) return;
	if (!XUI::BeginDialog(kTitlePackage, &s_WantPackage, XUI::DialogSize::M))
		return;
	XUI::BeginDialogBody();
	const bool requires_ok = !s_PkgRequires[0] || DottedNumber(s_PkgRequires, 3, 3);
	if (XUI::BeginPropertyGrid("package_dialog", .32f))
	{
		if (XUI::PropertyRow("Release", "The module id and the version from Mod Settings.", XUI::PropReadOnly))
			XUI::PropertyText((s_PkgManifest.id + "  " + s_PkgManifest.version).c_str());
		if (XUI::PropertyRow("Output Folder", "Receives <id>.update.ltx, the file index and the package zip(s). Inside the project "
			"only releases\\ is allowed: anywhere else the packages would ship with the next build.", XUI::PropRequired))
			XUI::PathField("##out_dir", s_PkgOutDir, sizeof(s_PkgOutDir), true);
		if (XUI::PropertyRow("Requires Game", "The lowest Dead Air: Refined version this release runs on. Empty: any.",
			requires_ok ? 0 : XUI::PropInvalid))
		{
			ImGui::SetNextItemWidth(_min(ImGui::CalcItemWidth(), XUI::Em(8.f)));
			ImGui::InputTextWithHint("##requires", "Any", s_PkgRequires, sizeof(s_PkgRequires));
			if (!requires_ok)
				XUI::PropertyMessage(XUI::Tone::Error, "MAJOR.MINOR.PATCH, e.g. 1.4.1.");
		}
		// read-only unless the key asks for its password: a locked key only leaves the release unsigned here
		if (XUI::PropertyRow("Author Key", "The key file that signs the release: the Mod Browser lists signed releases only. Submit to "
				"Mod Browser creates one.", EditorCatalog::KeyWantsPassword() ? 0 : XUI::PropReadOnly))
			EditorCatalog::DrawKeyLine("package");
		XUI::EndPropertyGrid();
	}
	if (s_PkgManifest.update_github.empty())
		XUI::StatusText(XUI::Tone::Warning, "No GitHub repository in Mod Settings: the game will not find this release.");

	if (XUI::BeginDialogFooter())
	{
		const char* why = s_PackageQueued ? "A packaging is running" : !s_PkgOutDir[0] ? "Pick the output folder first" : "Fix the game version first";
		if (XUI::DialogButton("Package", XUI::Kind::Primary, true, s_PkgOutDir[0] && requires_ok && !s_PackageQueued, why))
		{
			// An earlier packaging of this version is replaced - but only once the author has seen which files that is.
			string_path dir;
			xr_string ignored;
			xr_vector<xr_string> old_files;
			if (ResolveReleaseDir(EditorProject::Root(), s_PkgOutDir, s_PkgManifest.version, dir, ignored))
				ListReleaseFiles(dir, s_PkgManifest.id, s_PkgManifest.version, old_files);
			if (old_files.empty())
				QueuePackage();
			else
			{
				const xr_string headline = NqUtil::Format("An earlier packaging of %s is in %s.", s_PkgManifest.version.c_str(), dir);
				XUI::ConfirmSpec spec;
				spec.id = "replace_release";
				spec.title = "Replace Release Files";
				spec.headline = headline.c_str();
				spec.items = old_files;
				spec.consequence = "Deleted for good - this cannot be undone. The new packaging takes their place.";
				spec.action = "Replace";
				spec.destructive = true;
				XUI::Confirm(spec, [](XUI::Answer answer) { if (answer == XUI::Answer::Yes) QueuePackage(); });
			}
		}
		if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
			s_WantPackage = false;
	}
	XUI::EndDialog();
}

void EditorMod::DrawUI()
{
	if (!EditorProject::Active()) return;
	if (s_WantBuild && !s_BuildQueued)
		s_BuildQueued = UI->DeferUIWork(&RunBuildIntoGame);
	if (s_SettingsOpen)
		ShowSettingsTab(false);
	DrawExportDialog(false);
	DrawExportDialog(true);
	DrawPackageDialog();
}

//------------------------------------------------------------------------------
// MCP commands
//------------------------------------------------------------------------------
void EditorMod::McpManifest(xr_string& out)
{
	if (!EditorProject::Active())
		{ out = "{\"ok\":false,\"error\":\"no active project\"}"; return; }
	SManifest m;
	if (!Load(EditorProject::Root(), m))
		{ out = "{\"ok\":false,\"error\":\"mod.ltx not found in the project\"}"; return; }

	out = "{\"ok\":true,\"id\":\"";
	JsonAppend(out, m.id.c_str());
	out += "\",\"name\":\"";
	JsonAppend(out, m.name.c_str());
	out += "\",\"version\":\"";
	JsonAppend(out, m.version.c_str());
	out += "\",\"author\":\"";
	JsonAppend(out, m.author.c_str());
	out += "\",\"description\":\"";
	JsonAppend(out, m.description.c_str());
	out += "\",\"website\":\"";
	JsonAppend(out, m.website.c_str());
	out += "\",\"github\":\"";
	JsonAppend(out, m.update_github.c_str());
	out += "\",\"api\":\"";
	JsonAppend(out, m.api.c_str());
	out += "\",\"target\":\"";
	JsonAppend(out, m.target.c_str());
	out += "\",\"mode\":\"";
	JsonAppend(out, m.mode.c_str());
	out += "\",\"provides_mode\":[";
	for (u32 i = 0; i < m.provides_modes.size(); ++i)
	{
		if (i) out += ",";
		out += "{\"id\":\"";
		JsonAppend(out, m.provides_modes[i].id.c_str());
		out += "\",\"title\":\"";
		JsonAppend(out, m.provides_modes[i].title.c_str());
		out += "\"}";
	}
	out += "],\"requires\":";
	JsonAppendArray(out, m.requires_list);
	out += ",\"after\":";
	JsonAppendArray(out, m.after);
	out += ",\"before\":";
	JsonAppendArray(out, m.before);
	out += ",\"conflicts\":";
	JsonAppendArray(out, m.conflicts);
	out += ",\"exclude\":";
	JsonAppendArray(out, m.exclude);
	out += ",\"thumbnail\":\"";
	JsonAppendPath(out, m.thumbnail.c_str());
	out += "\",\"tags\":";
	JsonAppendArray(out, m.tags);
	out += ",\"world\":";
	EditorWorld::JsonWorld(m.world, out);
	out += ",\"pda_layout\":\"";
	JsonAppend(out, m.pda_layout.c_str());
	out += "\",\"has_dirs\":{";
	static const char* dirs[] = { "gamedata", "patch", "spawn", "scripts", "levels" };
	for (int i = 0; i < (int)(sizeof(dirs) / sizeof(dirs[0])); ++i)
	{
		char p[MAX_PATH];
		sprintf_s(p, "%s\\%s", EditorProject::Root(), dirs[i]);
		char item[64];
		sprintf_s(item, "%s\"%s\":%s", i ? "," : "", dirs[i], DirExists(p) ? "true" : "false");
		out += item;
	}
	out += "}}";
}

void EditorMod::McpSetManifest(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active())
		{ out = "{\"ok\":false,\"error\":\"no active project\"}"; return; }
	SManifest m;
	if (!Load(EditorProject::Root(), m))
	{
		// a pre-XMS project: scaffold the manifest, then edit it
		EnsureScaffold(EditorProject::Root(), EditorProject::Name());
		Load(EditorProject::Root(), m);
	}

	// GetArg answers an oversized value like a missing one. A buffer the size of
	// the request is never too small, so a long value is judged, not dropped.
	xr_vector<char> arg(xr_strlen(raw) + 1);
	char* const v = arg.data();
	const u32 v_size = (u32)arg.size();
	if (XFinedMCP::GetArg(raw, "id", v, v_size))
	{
		if (!ValidateId(v))
			{ out = "{\"ok\":false,\"error\":\"invalid id (allowed: a-z 0-9 _ . -)\"}"; return; }
		m.id = v;
	}
	if (XFinedMCP::GetArg(raw, "name", v, v_size))		m.name = v;
	if (XFinedMCP::GetArg(raw, "version", v, v_size))
	{
		if (!ValidateVersion(v))
			{ out = "{\"ok\":false,\"error\":\"invalid version (1-4 numbers separated by dots, e.g. 1.2.0)\"}"; return; }
		m.version = v;
	}
	if (XFinedMCP::GetArg(raw, "author", v, v_size))		m.author = v;
	if (XFinedMCP::GetArg(raw, "description", v, v_size))
	{
		m.description = v;
		Trim(m.description);
		// the same room the manifest window gives it
		if (m.description.size() >= sizeof(SSettingsForm::description))
			{ out = "{\"ok\":false,\"error\":\"description is too long (4095 bytes at most)\"}"; return; }
	}
	if (XFinedMCP::GetArg(raw, "website", v, v_size))
	{
		m.website = v;
		Trim(m.website);
		if (!ValidateWebsite(m.website.c_str()))
			{ out = "{\"ok\":false,\"error\":\"invalid website (only https://ap-pro.ru or https://moddb.com links are allowed)\"}"; return; }
	}
	if (XFinedMCP::GetArg(raw, "github", v, v_size))
	{
		NormalizeGithubRepo(v, m.update_github);
		if (!ValidateGithubRepo(m.update_github.c_str()))
			{ out = "{\"ok\":false,\"error\":\"invalid github (owner/repo, e.g. Madmer/agroprom-story)\"}"; return; }
	}
	if (XFinedMCP::GetArg(raw, "mode", v, v_size))
	{
		if (!ValidateModeId(v))
			{ out = "{\"ok\":false,\"error\":\"invalid mode id (allowed: a-z 0-9 _ . - or empty)\"}"; return; }
		m.mode = v;
	}
	// provides_mode edits touch only the first declared mode; empty id drops it
	if (XFinedMCP::GetArg(raw, "provides_mode_id", v, v_size))
	{
		if (!ValidateModeId(v))
			{ out = "{\"ok\":false,\"error\":\"invalid provides_mode_id (allowed: a-z 0-9 _ . - or empty)\"}"; return; }
		if (v[0])
		{
			if (m.provides_modes.empty()) m.provides_modes.push_back(SProvidesMode());
			m.provides_modes[0].id = v;
		}
		else if (!m.provides_modes.empty())
			m.provides_modes.erase(m.provides_modes.begin());
	}
	if (XFinedMCP::GetArg(raw, "provides_mode_title", v, v_size) && !m.provides_modes.empty())
		m.provides_modes[0].title = v;
	// The targeting choice itself. "default" cannot be inferred - a module with
	// no mode set either targets the ordinary game on purpose or was never
	// decided, and only the caller knows which - so it is stated explicitly;
	// picking a mode states it implicitly.
	if (XFinedMCP::GetArg(raw, "target", v, v_size))
	{
		if (0 != _stricmp(v, "default") && 0 != _stricmp(v, "existing") && 0 != _stricmp(v, "new"))
			{ out = "{\"ok\":false,\"error\":\"invalid target (allowed: default, existing, new)\"}"; return; }
		m.target = v;
	}
	else if (!m.provides_modes.empty())	m.target = "new";
	else if (!m.mode.empty())			m.target = "existing";
	if (XFinedMCP::GetArg(raw, "requires", v, v_size))	SplitList(m.requires_list, v, ',');
	if (XFinedMCP::GetArg(raw, "after", v, v_size))		SplitList(m.after, v, ',');
	if (XFinedMCP::GetArg(raw, "before", v, v_size))		SplitList(m.before, v, ',');
	if (XFinedMCP::GetArg(raw, "conflicts", v, v_size))	SplitList(m.conflicts, v, ',');
	// the whole list at once: one entry per line, or comma separated when on one line
	if (XFinedMCP::GetArg(raw, "exclude", v, v_size))
		SplitList(m.exclude, v, strchr(v, '\n') ? '\n' : ',');
	// the Mod Browser's picture (converted at once) and tags
	if (XFinedMCP::GetArg(raw, "tags", v, v_size))
	{
		xr_string err;
		if (!ApplyTags(v, m, err))
		{
			out = "{\"ok\":false,\"error\":\"";
			JsonAppend(out, err.c_str());
			out += "\"}";
			return;
		}
	}
	if (XFinedMCP::GetArg(raw, "thumbnail", v, v_size))
	{
		xr_string err;
		if (!ApplyThumbnail(EditorProject::Root(), v, m, err))
		{
			out = "{\"ok\":false,\"error\":\"";
			JsonAppend(out, err.c_str());
			out += "\"}";
			return;
		}
	}

	if (!ValidateId(m.id.c_str()))
		{ out = "{\"ok\":false,\"error\":\"module id missing or invalid (allowed: a-z 0-9 _ . -)\"}"; return; }
	if (!Save(EditorProject::Root(), m))
		{ out = "{\"ok\":false,\"error\":\"can't write mod.ltx\"}"; return; }
	McpManifest(out);	// respond with the fresh state
}

void EditorMod::McpExport(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active())
		{ out = "{\"ok\":false,\"error\":\"no active project\"}"; return; }
	char target[MAX_PATH];
	// no target = deploy to the linked game: <game>\modules\<id> is where the
	// game actually reads modules from, so it is the default, not an option
	if (!XFinedMCP::GetArg(raw, "target", target, sizeof(target)) || !target[0])
	{
		if (!EditorProject::GameRoot()[0])
			{ out = "{\"ok\":false,\"error\":\"no 'target' and no linked game (see game_link)\"}"; return; }
		strncpy_s(target, sizeof(target), EditorProject::GameRoot(), _TRUNCATE);
	}
	const bool flat = ArgBool(raw, "flat", false);
	xr_string bake_note;
	RunPreBuildBake(bake_note);

	int files = 0;
	xr_string path, err, warn;
	xr_vector<xr_string> left_out;
	// MCP is the programmatic path: the caller already decided, and it must
	// never stall on a question
	if (!Export(EditorProject::Root(), target, flat, files, path, err, true, &warn, &left_out))
	{
		out = "{\"ok\":false,\"error\":\"";
		JsonAppend(out, err.c_str());
		out += "\"}";
		return;
	}
	char head[64];
	sprintf_s(head, "{\"ok\":true,\"files\":%d,\"path\":\"", files);
	out = head;
	JsonAppendPath(out, path.c_str());
	out += "\",\"baked\":\"";
	JsonAppend(out, bake_note.c_str());
	// an agent gets the same warnings the dialog shows: a build that shipped a quest
	// naming something that does not exist is not a clean build
	out += "\",\"warnings\":\"";
	JsonAppend(out, warn.c_str());
	// what the build passed by - tooling, [exclude], links - project-relative
	out += "\",\"left_out\":";
	JsonAppendArray(out, left_out);
	out += "}";
}

void EditorMod::McpExclude(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active())
		{ out = "{\"ok\":false,\"error\":\"no active project\"}"; return; }
	char path[MAX_PATH] = {};
	if (!XFinedMCP::GetArg(raw, "path", path, sizeof(path)) || !path[0])
		{ out = "{\"ok\":false,\"error\":\"missing 'path' (project-relative, e.g. docs or design/story.md)\"}"; return; }
	// the raw json value still carries its escaped separators
	{
		char* w = path;
		for (const char* r = path; *r; ++r)
			if (!(*r == '\\' && w > path && w[-1] == '\\')) *w++ = *r;
		*w = 0;
	}
	// true unless told otherwise - the menu item it stands for excludes
	const bool excluded = NqUtil::ArgBool(raw, "excluded", true);
	xr_string err;
	if (!SetExcluded(EditorProject::Root(), path, excluded, err))
	{
		out = "{\"ok\":false,\"error\":\"";
		JsonAppend(out, err.c_str());
		out += "\"}";
		return;
	}
	out = "{\"ok\":true,\"exclude\":";
	JsonAppendArray(out, Exclusions(EditorProject::Root()));
	out += "}";
}

void EditorMod::McpPackage(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active())
		{ out = "{\"ok\":false,\"error\":\"no active project\"}"; return; }
	// request-sized, so an over-long value reaches the checks instead of reading as absent
	xr_vector<char> arg(xr_strlen(raw) + 1);
	xr_string out_dir, game_version;
	if (XFinedMCP::GetArg(raw, "out_dir", arg.data(), (u32)arg.size()))
		out_dir = arg.data();
	// absent = the linked game's version, present but empty = no requirement
	if (XFinedMCP::GetArg(raw, "requires_game", arg.data(), (u32)arg.size()))
		game_version = arg.data();
	else
		LinkedGameVersion(game_version);

	// the author key's password, when the key file has one; never echoed or logged
	if (XFinedMCP::GetArg(raw, "password", arg.data(), (u32)arg.size()) && arg[0])
	{
		xr_string key_err;
		const bool opened = EditorCatalog::Unlock(arg.data(), key_err);
		::SecureZeroMemory(arg.data(), arg.size());
		if (!opened)
		{
			out = "{\"ok\":false,\"error\":\"author key: ";
			JsonAppend(out, key_err.c_str());
			out += "\"}";
			return;
		}
	}

	// MCP is the programmatic path: it asks nothing, an earlier packaging of the
	// same version is simply replaced
	SProgressOperation progress(*UI, true);
	xr_string bake_note;
	RunPreBuildBake(bake_note);
	SPackageResult res;
	xr_string err, warn;
	if (!PackageRelease(EditorProject::Root(), out_dir.c_str(), game_version.c_str(), res, err, &warn))
	{
		out = "{\"ok\":false,\"error\":\"";
		JsonAppend(out, err.c_str());
		out += "\"}";
		return;
	}
	out = "{\"ok\":true,\"descriptor\":\"";
	JsonAppendPath(out, res.descriptor.c_str());
	out += "\",\"out_dir\":\"";
	JsonAppendPath(out, res.out_dir.c_str());
	out += "\",\"index\":{\"name\":\"";
	JsonAppend(out, res.index.name.c_str());
	out += NqUtil::Format("\",\"size\":%llu,\"sha256\":\"", res.index.size);
	out += res.index.sha256;
	out += "\"},\"packages\":[";
	for (u32 i = 0; i < res.packages.size(); ++i)
	{
		if (i) out += ",";
		out += "{\"name\":\"";
		JsonAppend(out, res.packages[i].name.c_str());
		out += NqUtil::Format("\",\"size\":%llu,\"sha256\":\"", res.packages[i].size);
		out += res.packages[i].sha256;
		out += "\"}";
	}
	out += NqUtil::Format("],\"files\":%d,\"unpacked\":%llu,\"packed\":%llu,\"requires_game\":\"",
		res.files, res.unpacked, res.packed);
	JsonAppend(out, res.requires_game.c_str());
	out += "\",\"baked\":\"";
	JsonAppend(out, bake_note.c_str());
	out += "\",\"warnings\":\"";
	JsonAppend(out, warn.c_str());
	out += NqUtil::Format("\",\"signed\":%s,\"key_id\":\"%s\",\"left_out\":", res.signed_ ? "true" : "false", res.key_id.c_str());
	JsonAppendArray(out, res.left_out);
	// the Mod Browser's content rules: reported, the release is written either way
	out += ",\"catalog_content\":[";
	for (u32 i = 0; i < res.catalog_block.size() + res.catalog_review.size(); ++i)
	{
		const bool block = i < res.catalog_block.size();
		out += i ? ",{\"level\":\"" : "{\"level\":\"";
		out += block ? "block\",\"text\":\"" : "review\",\"text\":\"";
		JsonAppend(out, (block ? res.catalog_block[i] : res.catalog_review[i - res.catalog_block.size()]).c_str());
		out += "\"}";
	}
	out += "]}";
}

static LPCSTR ToneName(XUI::Tone tone)
{
	switch (tone)
	{
	case XUI::Tone::Error:		return "error";
	case XUI::Tone::Required:	return "required";
	case XUI::Tone::Warning:	return "warning";
	default:					return "note";
	}
}

void EditorMod::McpSettingsView(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active())
		{ out = "{\"ok\":false,\"error\":\"no active project\"}"; return; }
	// a page opens the tab on it; without one this only looks
	xr_string page;
	if (NqUtil::ArgString(raw, "page", page) && !page.empty())
	{
		int index = -1;
		for (int i = 0; i < pgCount; ++i)
			if (page == kSettingsPages[i].id)
				index = i;
		if (index < 0)
		{
			xr_string err = "page must be one of:";
			for (int i = 0; i < pgCount; ++i)
				err += xr_string(i ? ", " : " ") + kSettingsPages[i].id;
			NqUtil::JsonError(out, err);
			return;
		}
		RequestEditManifest();
		s_SettingsPage = index;
		if (index == pgPda) RefreshPdaFiles();
		if (index == pgRelations) s_InstalledScanned = false;
	}
	if (s_SettingsOpen)
	{
		SyncSettingsForm();
		CheckSettings();
	}
	else
		s_SettingsProblems.clear();

	out = "{\"ok\":true,\"tab\":\"mod_settings\",\"open\":";
	NqUtil::JsonBool(out, s_SettingsOpen);
	out += ",\"active\":";
	NqUtil::JsonBool(out, XUI::Shell::IsActive("mod_settings"));
	out += ",\"page\":";
	NqUtil::JsonString(out, kSettingsPages[s_SettingsPage].id);
	out += ",\"dirty\":";
	NqUtil::JsonBool(out, s_SettingsOpen && SettingsDirty());
	out += ",\"pages\":[";
	for (int i = 0; i < pgCount; ++i)
	{
		XUI::Tone tone = XUI::Tone::Muted;
		out += i ? ",{\"id\":" : "{\"id\":";
		NqUtil::JsonString(out, kSettingsPages[i].id);
		out += ",\"label\":";
		NqUtil::JsonString(out, kSettingsPages[i].label);
		out += ",\"state\":";
		NqUtil::JsonString(out, PageTone(i, tone) ? ToneName(tone) : "ok");
		out += "}";
	}
	out += "],\"problems\":[";
	for (u32 i = 0; i < s_SettingsProblems.size(); ++i)
	{
		const SSettingsProblem& p = s_SettingsProblems[i];
		out += i ? ",{\"page\":" : "{\"page\":";
		NqUtil::JsonString(out, kSettingsPages[p.page].id);
		out += ",\"field\":";
		NqUtil::JsonString(out, p.field);
		out += ",\"tone\":";
		NqUtil::JsonString(out, ToneName(p.tone));
		out += ",\"blocks_save\":";
		NqUtil::JsonBool(out, p.blocks);
		out += ",\"text\":";
		NqUtil::JsonString(out, p.text);
		out += "}";
	}
	out += "]}";
}
