#include "stdafx.h"
#include "EditorGameContent.h"
#include "EditorGameModes.h"
#include "EditorModManifest.h"
#include "EditorUiContract.h"
#include "EditorCatalog.h"
#include "EditorCharacters.h"
#include "EditorWorld.h"
#include "EditorScreens.h"
#include "EditorPda.h"
#include "EditorProject.h"
#include "GrizzlyMCP.h"
#include "EDX11Utils.h"
#include "ui_main.h"
#include "Nq/NqExport.h"
#include "Nq/NqDoc.h"
#include "Nq/NqUtil.h"
#include "UI_ProgressCenter.h"
#include <atomic>
#include <mutex>
#include <thread>
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

// ---- [screens], [screen.<id>], [track.<id>] ----------------------------------------------------
namespace EditorMod
{
static u32 ScreenSurfaces(const xr_string& value)
{
	xr_vector<xr_string> names;
	NqUtil::Split(value.c_str(), ',', names);
	u32 mask = 0;
	for (xr_string& name : names)
	{
		Trim(name);
		if (name == "loading")		mask |= scrLoading;
		else if (name == "menu")	mask |= scrMenu;
		else if (name == "pause")	mask |= scrPause;
	}
	return mask;
}

static xr_string ScreenSurfaceText(u32 mask)
{
	xr_string out;
	if (mask & scrLoading)	out += "loading";
	if (mask & scrMenu)		out += out.empty() ? "menu" : ", menu";
	if (mask & scrPause)	out += out.empty() ? "pause" : ", pause";
	return out;
}

static void ScreenList(const xr_string& value, xr_vector<xr_string>& out)
{
	out.clear();
	NqUtil::Split(value.c_str(), ',', out);
	for (xr_string& v : out)
		Trim(v);
}

static void ParseScreensKey(SScreens& s, const xr_string& key, const xr_string& val)
{
	const bool only = val == "only", next = val == "next";
	if (key == "loading_screens")			s.only_screens[0] = only;
	else if (key == "menu_screens")			s.only_screens[1] = only;
	else if (key == "loading_music")		s.only_music[0] = only;
	else if (key == "menu_music")			s.only_music[1] = only;
	else if (key == "loading_music_end")	s.next_track[0] = next;
	else if (key == "menu_music_end")		s.next_track[1] = next;
}

static void ParseScreenKey(SScreen& s, const xr_string& key, const xr_string& val)
{
	if (key == "layer")
	{
		SScreenLayer layer;
		layer.file = val;
		s.layers.push_back(layer);
	}
	else if (key == "show")		s.show = ScreenSurfaces(val);
	else if (key == "levels")	ScreenList(val, s.levels);
	else if (key == "modes")	ScreenList(val, s.modes);
	else if (key == "weight")	s.weight = float(atof(val.c_str()));
	else if (key == "music")
	{
		s.tracks.clear();
		if (val.empty() || val == "any")			s.music = "any";
		else if (val == "none" || val == "video")	s.music = val;
		else
		{
			s.music = "tracks";
			ScreenList(val, s.tracks);
		}
	}
	else if (key == "hide")
	{
		xr_vector<xr_string> names;
		ScreenList(val, names);
		s.hide = 0;
		for (const xr_string& name : names)
		{
			if (name == "bar")			s.hide |= scrHideBar;
			else if (name == "percent")	s.hide |= scrHidePercent;
			else if (name == "stage")	s.hide |= scrHideStage;
			else if (name == "tip")		s.hide |= scrHideTip;
		}
	}
	else if (!s.layers.empty())
	{
		SScreenLayer& layer = s.layers.back();
		if (key == "fit")			layer.fit = val == "contain" ? scrContain : val == "stretch" ? scrStretch : scrCover;
		else if (key == "opacity")	layer.opacity = float(atof(val.c_str()));
		else if (key == "rect")		sscanf_s(val.c_str(), "%f %f %f %f", &layer.x, &layer.y, &layer.w, &layer.h);
		else if (key == "texture")	layer.texture = val;
		else if (key == "sound")	layer.sound = !(val == "false" || val == "0" || val == "no" || val == "off");
	}
}

static void ParseTrackKey(STrack& t, const xr_string& key, const xr_string& val)
{
	if (key == "file")			t.file = val;
	else if (key == "play")		t.play = ScreenSurfaces(val);
	else if (key == "weight")	t.weight = float(atof(val.c_str()));
	else if (key == "volume")	t.volume = float(atof(val.c_str()));
}

static xr_string ScreenNumber(float v)
{
	char num[32];
	sprintf_s(num, "%g", v);
	return num;
}

static void AppendScreens(xr_string& t, const SScreens& s)
{
	const auto row = [&t](LPCSTR key, const xr_string& value)
	{
		xr_string line = key;
		if (line.size() < 17) line.append(17 - line.size(), ' ');
		t += line + " = " + value + "\r\n";
	};
	const auto joined = [](const xr_vector<xr_string>& v)
	{
		xr_string out;
		for (const xr_string& e : v)
			out += (out.empty() ? "" : ", ") + e;
		return out;
	};

	if (s.only_screens[0] || s.only_screens[1] || s.only_music[0] || s.only_music[1] || s.next_track[0] || s.next_track[1])
	{
		t += "\r\n[screens]\r\n";
		if (s.only_screens[0])	row("loading_screens", "only");
		if (s.only_music[0])	row("loading_music", "only");
		if (s.only_screens[1])	row("menu_screens", "only");
		if (s.only_music[1])	row("menu_music", "only");
		if (s.next_track[0])	row("loading_music_end", "next");
		if (s.next_track[1])	row("menu_music_end", "next");
	}
	for (const SScreen& d : s.screens)
	{
		t += "\r\n[screen." + d.id + "]\r\n";
		row("show", ScreenSurfaceText(d.show));
		if (!d.levels.empty())		row("levels", joined(d.levels));
		if (!d.modes.empty())		row("modes", joined(d.modes));
		if (d.weight != 1.f)		row("weight", ScreenNumber(d.weight));
		if (d.music == "tracks")	row("music", joined(d.tracks));
		else if (d.music != "any")	row("music", d.music);
		if (d.hide)
		{
			xr_vector<xr_string> names;
			if (d.hide & scrHideBar)		names.push_back("bar");
			if (d.hide & scrHidePercent)	names.push_back("percent");
			if (d.hide & scrHideStage)		names.push_back("stage");
			if (d.hide & scrHideTip)		names.push_back("tip");
			row("hide", joined(names));
		}
		for (const SScreenLayer& l : d.layers)
		{
			row("layer", l.file);
			if (l.fit == scrContain)		row("fit", "contain");
			else if (l.fit == scrStretch)	row("fit", "stretch");
			if (l.opacity != 1.f)			row("opacity", ScreenNumber(l.opacity));
			if (l.x != 0.f || l.y != 0.f || l.w != 100.f || l.h != 100.f)
				row("rect", ScreenNumber(l.x) + " " + ScreenNumber(l.y) + " " + ScreenNumber(l.w) + " " + ScreenNumber(l.h));
			if (!l.texture.empty())			row("texture", l.texture);
			if (!l.sound)					row("sound", "false");
		}
	}
	for (const STrack& d : s.tracks)
	{
		t += "\r\n[track." + d.id + "]\r\n";
		row("file", d.file);
		if (d.play)				row("play", ScreenSurfaceText(d.play));
		if (d.weight != 1.f)	row("weight", ScreenNumber(d.weight));
		if (d.volume != 1.f)	row("volume", ScreenNumber(d.volume));
	}
}

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

	enum ESec { secNone, secScreens, secScreen, secTrack, secModule, secRequires, secOrder, secConflicts, secProvides, secUpdate, secExclude, secWorld, secStartKit, secPda, secVars,
		secCommands, secCommand, secExtra };
	ESec sec = secNone;
	// the command a [command.<name>] section describes: the first of that name, or one made for it
	u32 command = 0;

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
			else if (sn == "start_kit")		{ sec = secStartKit; m.start_kit.declared = true; }
			else if (sn == "pda")			sec = secPda;
			else if (sn == "vars")			sec = secVars;
			else if (sn == "commands")		sec = secCommands;
			else if (sn == "screens")		sec = secScreens;
			else if (sn.size() > 7 && 0 == sn.compare(0, 7, "screen."))
			{
				sec = secScreen;
				SScreen s;
				s.id = sn.substr(7);
				m.screens.screens.push_back(s);
			}
			else if (sn.size() > 6 && 0 == sn.compare(0, 6, "track."))
			{
				sec = secTrack;
				STrack s;
				s.id = sn.substr(6);
				m.screens.tracks.push_back(s);
			}
			else if (sn.size() > 8 && 0 == sn.compare(0, 8, "command."))
			{
				sec = secCommand;
				const xr_string name = sn.substr(8);
				for (command = 0; command < m.commands.size(); ++command)
					if (0 == _stricmp(m.commands[command].name.c_str(), name.c_str())) break;
				if (command == m.commands.size())
				{
					SCommand c;
					c.name = name;
					c.declared = false;
					m.commands.push_back(c);
				}
			}
			else { sec = secExtra; m.extra.push_back(raw); }
			continue;
		}
		if (sec == secExtra) { if (!raw.empty()) m.extra.push_back(raw); continue; }
		if (line.empty()) continue;

		if (sec == secScreens || sec == secScreen || sec == secTrack)
		{
			xr_string key = line, val;
			const size_t eq = line.find('=');
			if (eq != xr_string::npos) { key = line.substr(0, eq); val = line.substr(eq + 1); }
			Trim(key); Trim(val);
			if (sec == secScreens)		ParseScreensKey(m.screens, key, val);
			else if (sec == secScreen)	ParseScreenKey(m.screens.screens.back(), key, val);
			else						ParseTrackKey(m.screens.tracks.back(), key, val);
			continue;
		}

		if (sec == secCommands)
		{
			// every line is a command of its own: a name declared twice stays twice, for the check to name it
			const size_t eq = line.find('=');
			xr_string name = eq == xr_string::npos ? line : line.substr(0, eq);
			xr_string value = eq == xr_string::npos ? xr_string() : line.substr(eq + 1);
			Trim(name); Trim(value);
			if (name.empty()) continue;
			// a [command.<name>] section read before its line already made the entry
			bool merged = false;
			for (SCommand& c : m.commands)
				if (!c.declared && 0 == _stricmp(c.name.c_str(), name.c_str()))
				{
					c.name = name;
					c.function = value;
					c.declared = true;
					merged = true;
					break;
				}
			if (!merged)
			{
				SCommand c;
				c.name = name;
				c.function = value;
				m.commands.push_back(c);
			}
			continue;
		}
		if (sec == secCommand)
		{
			xr_string key = line, val;
			const size_t eq = line.find('=');
			if (eq != xr_string::npos) { key = line.substr(0, eq); val = line.substr(eq + 1); }
			Trim(key); Trim(val);
			SCommand& c = m.commands[command];
			if      (key == "help")		{ if (!Unquote(val, c.help)) c.help = val; }
			else if (key == "args")		{ if (!Unquote(val, c.args)) c.args = val; }
			else if (key == "cheat")	c.cheat = !_stricmp(val.c_str(), "true") || val == "1" || !_stricmp(val.c_str(), "yes") || !_stricmp(val.c_str(), "on");
			else						c.extra.push_back(line);
			continue;
		}

		if (sec == secVars)
		{
			// the default stays as written: the quests read its type from it
			SGlobalVar gv;
			const size_t eq = line.find('=');
			gv.name = eq == xr_string::npos ? line : line.substr(0, eq);
			if (eq != xr_string::npos) gv.value = line.substr(eq + 1);
			Trim(gv.name); Trim(gv.value);
			if (!gv.name.empty()) m.vars.push_back(gv);
			continue;
		}
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
				else if (key == "disable_game_updates")	m.disable_game_updates = val == "true" || val == "1";
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
		else if (sec == secStartKit)
		{
			// DA's own loadout format (plugins\faction_loadouts.ltx): a bare section is one
			// of it, `section = N` is N, money and perks are the two words that are not items
			xr_string key = line, val;
			const size_t eq = line.find('=');
			if (eq != xr_string::npos) { key = line.substr(0, eq); val = line.substr(eq + 1); }
			Trim(key); Trim(val);
			SStartKit& k = m.start_kit;
			if (key == "money")			k.money = _max(0, atoi(val.c_str()));
			else if (key == "perks")	k.perks = !(val == "false" || val == "0" || val == "no" || val == "off");
			else if (!key.empty())
			{
				SKitItem it;
				it.section = key;
				it.count = val.empty() ? 1 : atoi(val.c_str());
				k.items.push_back(it);
			}
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
	if (m.disable_game_updates)
		AppendKey(t, "disable_game_updates", "true");
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
	// what a new game of the mode begins with: money and perks, then the items in the
	// author's order. An item section may be longer than AppendKey's pad buffer takes.
	if (m.start_kit.declared)
	{
		const SStartKit& k = m.start_kit;
		t += "\r\n[start_kit]\r\n";
		auto line = [&t](const xr_string& key, int value)
		{
			xr_string row = key;
			if (row.size() < (size_t)kWorldKeyWidth) row.append(kWorldKeyWidth - row.size(), ' ');
			char num[16];
			sprintf_s(num, "%d", value);
			t += row + " = " + num + "\r\n";
		};
		if (k.money >= 0)	line("money", k.money);
		if (!k.perks)		AppendKey(t, "perks", "false", kWorldKeyWidth);
		for (u32 i = 0; i < k.items.size(); ++i)
			if (!k.items[i].section.empty()) line(k.items[i].section, k.items[i].count);
	}
	// the global variables every quest of the module shares
	if (!m.vars.empty())
	{
		t += "\r\n[vars]\r\n";
		for (u32 i = 0; i < m.vars.size(); ++i)
		{
			xr_string row = m.vars[i].name;
			if (row.size() < (size_t)kWorldKeyWidth) row.append(kWorldKeyWidth - row.size(), ' ');
			t += row + " = " + m.vars[i].value + "\r\n";
		}
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
	AppendScreens(t, m.screens);
	// the console commands: the list, then the details of each in a section of its own
	bool any_declared = false;
	for (const SCommand& c : m.commands)
		if (c.declared) any_declared = true;
	if (any_declared)
	{
		t += "\r\n[commands]\r\n";
		for (const SCommand& c : m.commands)
		{
			if (!c.declared) continue;
			xr_string row = c.name;
			if (row.size() < (size_t)kWorldKeyWidth) row.append(kWorldKeyWidth - row.size(), ' ');
			t += row + " = " + c.function + "\r\n";
		}
	}
	for (u32 i = 0; i < m.commands.size(); ++i)
	{
		const SCommand& c = m.commands[i];
		if (c.name.empty() || (c.help.empty() && c.args.empty() && !c.cheat && c.extra.empty())) continue;
		// a name declared twice has one section: the first entry's
		bool first = true;
		for (u32 k = 0; k < i && first; ++k)
			first = 0 != _stricmp(m.commands[k].name.c_str(), c.name.c_str());
		if (!first) continue;
		t += "\r\n[command." + c.name + "]\r\n";
		if (!c.help.empty())	AppendText(t, "help", c.help, false);
		if (!c.args.empty())	AppendText(t, "args", c.args, false);
		if (c.cheat)			AppendKey(t, "cheat", "true");
		for (u32 k = 0; k < c.extra.size(); ++k)
			{ t += c.extra[k]; t += "\r\n"; }
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

// The global variables as mod.ltx declares them now. A quest is validated on every edit, so
// this reads the file once per manifest generation (a save of the manifest bumps it) and per
// project, never per call.
static xr_vector<EditorMod::SGlobalVar>	s_GlobalVars;
static u32								s_GlobalsGen = u32(-1);
static xr_string						s_GlobalsRoot;

void EditorMod::InvalidateGlobals()
{
	s_GlobalsGen = u32(-1);
}

const xr_vector<EditorMod::SGlobalVar>& EditorMod::ProjectGlobals()
{
	xr_vector<SGlobalVar>& s_Vars = s_GlobalVars;
	u32& s_Gen = s_GlobalsGen;
	xr_string& s_Root = s_GlobalsRoot;
	const xr_string root = EditorProject::Active() ? xr_string(EditorProject::Root()) : xr_string();
	if (s_Gen == s_ManifestGeneration && s_Root == root) return s_Vars;
	s_Gen = s_ManifestGeneration;
	s_Root = root;
	s_Vars.clear();
	SManifest m;
	if (!root.empty() && Load(root.c_str(), m)) s_Vars = m.vars;
	return s_Vars;
}

bool EditorMod::ValidGlobalName(LPCSTR name)
{
	if (!name || !name[0]) return false;
	for (LPCSTR p = name; *p; ++p)
		if (!((*p >= 'a' && *p <= 'z') || (*p >= 'A' && *p <= 'Z') || (*p >= '0' && *p <= '9') || *p == '_'))
			return false;
	return true;
}

//------------------------------------------------------------------------------
// console commands ([commands]); the game half is XmsGame::RegisterModuleCommands
//------------------------------------------------------------------------------
static bool IdentStart(char c)	{ return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_'; }
static bool IdentChar(char c)	{ return IdentStart(c) || (c >= '0' && c <= '9'); }

// the game's limits: 64 for a name, 128 for <script>.<function>
bool EditorMod::ValidCommandName(LPCSTR name)
{
	if (!name || !name[0] || xr_strlen(name) > 64) return false;
	for (LPCSTR p = name; *p; ++p)
		if (!((*p >= 'a' && *p <= 'z') || (*p >= '0' && *p <= '9') || *p == '_'))
			return false;
	return true;
}

bool EditorMod::ValidCommandFunction(LPCSTR spec)
{
	if (!spec || xr_strlen(spec) > 128) return false;
	LPCSTR p = spec;
	for (int part = 0; part < 2; ++part)
	{
		if (!IdentStart(*p)) return false;
		while (IdentChar(*p)) ++p;
		if (part == 0 && *p++ != '.') return false;
	}
	return *p == 0;
}

// `function name(` or `name = function(` at the start of a line: what the script's table keeps.
// `local function` stays inside the script; `function a.b(` and `function a:b(` belong to a table.
static void ScanScriptFunctions(const xr_string& text, const xr_string& script, xr_vector<xr_string>& out)
{
	for (size_t pos = 0; pos < text.size();)
	{
		size_t nl = text.find('\n', pos);
		if (nl == xr_string::npos) nl = text.size();
		LPCSTR p = text.c_str() + pos;
		LPCSTR const e = text.c_str() + nl;
		pos = nl + 1;
		while (p < e && (*p == ' ' || *p == '\t')) ++p;
		const bool keyword = e - p > 9 && 0 == strncmp(p, "function", 8) && (p[8] == ' ' || p[8] == '\t');
		if (keyword) p += 9;
		while (p < e && (*p == ' ' || *p == '\t')) ++p;
		if (p >= e || !IdentStart(*p)) continue;
		LPCSTR const name = p;
		while (p < e && IdentChar(*p)) ++p;
		xr_string fn;
		fn.append(name, size_t(p - name));
		while (p < e && (*p == ' ' || *p == '\t')) ++p;
		if (!keyword)
		{
			if (p >= e || *p != '=' || (p + 1 < e && p[1] == '=')) continue;
			++p;
			while (p < e && (*p == ' ' || *p == '\t')) ++p;
			if (e - p < 8 || 0 != strncmp(p, "function", 8)) continue;
			p += 8;
			while (p < e && (*p == ' ' || *p == '\t')) ++p;
		}
		if (p < e && *p == '(')
			out.push_back(script + "." + fn);
	}
}

void EditorMod::ScriptFunctions(LPCSTR project_root, xr_vector<xr_string>& out)
{
	out.clear();
	if (!project_root || !project_root[0]) return;
	// the game's script namespaces are the files right in scripts\, never a folder below it
	string_path mask;
	sprintf_s(mask, "%s\\gamedata\\scripts\\*.script", project_root);
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (INVALID_HANDLE_VALUE == h) return;
	do
	{
		const xr_string file = fd.cFileName;
		if ((fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) || file.size() <= 7 || 0 != _stricmp(file.c_str() + file.size() - 7, ".script"))
			continue;
		const xr_string rel = "gamedata\\scripts\\" + file;
		if (OutsideBuild(project_root, rel.c_str())) continue;
		xr_string text;
		if (ReadTextFile((xr_string(project_root) + "\\" + rel).c_str(), text))
			ScanScriptFunctions(text, file.substr(0, file.size() - 7), out);
	} while (::FindNextFileA(h, &fd));
	::FindClose(h);
	std::sort(out.begin(), out.end());
	out.erase(std::unique(out.begin(), out.end()), out.end());
}

// The game finds the script by its file name, the function by its exact name.
static bool HasScriptFunction(const xr_vector<xr_string>& functions, const xr_string& spec)
{
	const size_t dot = spec.find('.');
	if (dot == xr_string::npos) return false;
	for (const xr_string& f : functions)
		if (f.find('.') == dot && 0 == _strnicmp(f.c_str(), spec.c_str(), dot) && f.compare(dot, xr_string::npos, spec, dot, xr_string::npos) == 0)
			return true;
	return false;
}

void EditorMod::InstalledCommands(LPCSTR self_id, xr_vector<std::pair<xr_string, xr_string> >& out)
{
	out.clear();
	LPCSTR game = EditorProject::GameRoot();
	if (!game || !game[0]) return;
	// the two roots the game reads modules from
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
			if (!(fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) || fd.cFileName[0] == '.') continue;
			string_path dir;
			sprintf_s(dir, "%s\\%s\\%s", game, root, fd.cFileName);
			SManifest other;
			if (!Load(dir, other)) continue;
			const xr_string id = other.id.empty() ? xr_string(fd.cFileName) : other.id;
			if (self_id && 0 == _stricmp(id.c_str(), self_id)) continue;
			for (const SCommand& c : other.commands)
				if (c.declared) out.emplace_back(c.name, id);
		} while (::FindNextFileA(h, &fd));
		::FindClose(h);
	}
}

// The game's own command families and short words a module could pick (the engine registers ~1000
// commands; these are the ones a mod's name runs into). A family only warns, a word is refused.
static bool StockCommandWord(LPCSTR name)
{
	static const char* kWords[] = { "bind", "chat", "disconnect", "flush", "fov", "help", "hide", "load", "name", "nq", "quit",
		"renderer", "save", "screenshot", "start", "unbind", "unbindall", "cfg_save", "cfg_load", "main_menu", "run_string",
		"time_factor", "default_controls" };
	for (LPCSTR w : kWords)
		if (0 == strcmp(name, w)) return true;
	return false;
}

static LPCSTR StockCommandFamily(LPCSTR name)
{
	static const char* kFamilies[] = { "xms_", "r_", "r1_", "r2_", "r3_", "r4_", "r__", "rs_", "g_", "sv_", "cl_", "net_", "dbg_",
		"debug_", "ai_", "al_", "ph_", "hud_", "snd_", "vid_", "mm_", "mt_", "dar_", "cam_", "ui_", "inv_", "lua_", "gamepad_",
		"demo_", "stat_", "bind_", "unbind_" };
	for (LPCSTR f : kFamilies)
		if (0 == strncmp(name, f, xr_strlen(f))) return f;
	return 0;
}

void EditorMod::ValidateCommands(LPCSTR project_root, const SManifest& m, xr_vector<xr_string>& problems)
{
	if (m.commands.empty()) return;
	xr_vector<xr_string> functions;
	ScriptFunctions(project_root, functions);
	xr_vector<std::pair<xr_string, xr_string> > taken;
	InstalledCommands(m.id.c_str(), taken);
	for (u32 i = 0; i < m.commands.size(); ++i)
	{
		const SCommand& c = m.commands[i];
		const xr_string where = "commands: '" + c.name + "'";
		if (!c.declared)
		{
			problems.push_back("E405 " + where + ": [command." + c.name + "] describes a command [commands] does not declare - "
				"add its line or remove the section");
			continue;
		}
		if (!ValidCommandName(c.name.c_str()))
			problems.push_back("E401 " + where + " is not a console command name - a-z, 0-9 and _ only, 64 at most");
		else if (StockCommandWord(c.name.c_str()))
			problems.push_back("E406 " + where + " is a command of the game - it keeps its own and refuses this one");
		else if (LPCSTR family = StockCommandFamily(c.name.c_str()))
			problems.push_back("W406 " + where + " is named like the game's own " + family + "* commands - if the game has "
				"one of that name, it keeps it and refuses this one");
		for (u32 k = 0; k < i; ++k)
			if (m.commands[k].declared && 0 == _stricmp(m.commands[k].name.c_str(), c.name.c_str()))
			{
				problems.push_back("E402 " + where + " is declared twice - the game takes the first line and refuses this one");
				break;
			}
		if (!ValidCommandFunction(c.function.c_str()))
			problems.push_back("E403 " + where + " calls '" + c.function + "' - it takes <script>.<function>, e.g. my_mod.status");
		else if (!HasScriptFunction(functions, c.function))
		{
			const xr_string script = c.function.substr(0, c.function.find('.'));
			problems.push_back("E404 " + where + ": gamedata\\scripts\\" + script + ".script has no function " +
				c.function.substr(script.size() + 1) + "( - the game would find nothing to call");
		}
		for (const std::pair<xr_string, xr_string>& t : taken)
			if (0 == _stricmp(t.first.c_str(), c.name.c_str()))
			{
				problems.push_back("W407 " + where + " is a command of module '" + t.second + "' of the linked game too - "
					"the one that loads first keeps it");
				break;
			}
		xr_string bad;
		if (!NqUtil::Cp1251Covers(c.help + c.args, bad))
			problems.push_back("W408 " + where + ": the help or the arguments hold " + bad + ", which the console shows as '?'");
	}
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
// What a job's thread reports, and what it is told. The thread touches nothing of the editor: the log belongs to the
// main thread as well, so its lines wait here.
enum { kStepCheck, kStepCopy, kStepFinish };

static struct SJobFeed
{
	std::mutex				lock;
	std::atomic<bool>		cancel			{ false };
	int						step			= kStepCheck;
	xr_string				stage, file;
	u64						done			= 0;
	u64						total			= 0;
	u32						files			= 0;
	u32						files_total		= 0;
	u32						counted_from	= 0;	// when `done` started to count: how fast it goes, how long is left
	xr_vector<xr_string>	log;

	bool Cancelled() const { return cancel; }

	void Stage(int at, LPCSTR text, u64 bytes = 0, u32 count = 0)
	{
		std::lock_guard<std::mutex> guard(lock);
		step			= at;
		stage			= text;
		file.clear();
		done			= 0;
		total			= bytes;
		files			= 0;
		files_total		= count;
		counted_from	= ::GetTickCount();
	}
	void Add(u64 bytes)
	{
		std::lock_guard<std::mutex> guard(lock);
		done += bytes;
	}
	void File(const xr_string& rel)
	{
		std::lock_guard<std::mutex> guard(lock);
		file = rel;
		++files;
	}
	void Note(const xr_string& line)
	{
		std::lock_guard<std::mutex> guard(lock);
		log.push_back(line);
	}
} s_Feed;

// What a build ships, as ONE list made before anything is written. A clean build
// has to know what it will write to tell what in the target it would throw away,
// and the author is told what the walk passed by: a folder that silently did not
// ship is the next bug report.
struct SBuildFile
{
	xr_string	src;		// absolute
	xr_string	rel;		// project-relative, '\\'
	u64			size	= 0;
	FILETIME	written	= {};
	bool		reuse	= false;	// the last build holds this very file
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
		if (s_Feed.Cancelled())	return;
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
			f.size = (u64(fd.nFileSizeHigh) << 32) | fd.nFileSizeLow;
			f.written = fd.ftLastWriteTime;
			files.push_back(f);
		} while (::FindNextFileA(h, &fd));
		::FindClose(h);
	}
};

static DWORD CALLBACK CopyProgress(LARGE_INTEGER, LARGE_INTEGER sent, LARGE_INTEGER, LARGE_INTEGER,
								   DWORD, DWORD, HANDLE, HANDLE, LPVOID data)
{
	u64& counted = *(u64*)data;
	s_Feed.Add(u64(sent.QuadPart) - counted);
	counted = u64(sent.QuadPart);
	return s_Feed.Cancelled() ? PROGRESS_CANCEL : PROGRESS_CONTINUE;
}

// Writes the walked files below `dst`; `strip` leading characters of each relative
// path are dropped (the flat overlay copies gamedata\\ into its own root)
// A file that cannot be written fails the build (`err`): a module short of a file would
// go out as a release whose index says it is complete. A file marked `reuse` is linked
// from `reuse_from`, where the last build left it: the build then costs neither the
// time nor the second copy on the disk.
static int CopyBuildFiles(const SBuildWalk& walk, LPCSTR dst, size_t strip, LPCSTR reuse_from, xr_string& err)
{
	int copied = 0;
	xr_string made;		// the last folder created - files come in folder order
	for (u32 i = 0; i < walk.files.size(); ++i)
	{
		if (s_Feed.Cancelled()) { err = "cancelled"; break; }
		const SBuildFile& f = walk.files[i];
		const xr_string rel = f.rel.substr(_min(strip, f.rel.size()));
		const xr_string to = xr_string(dst) + "\\" + rel;
		if (to.size() >= sizeof(string_path))
		{
			err = "the path is too long for the target folder: " + rel;
			break;
		}
		const size_t cut = to.rfind('\\');
		const xr_string folder = to.substr(0, cut);
		if (folder != made)
		{
			CreateDirChain(folder.c_str());
			made = folder;
		}
		s_Feed.File(rel);
		if (f.reuse && ::CreateHardLinkA(to.c_str(), (xr_string(reuse_from) + "\\" + rel).c_str(), NULL))
		{
			s_Feed.Add(f.size);
			++copied;
			continue;
		}
		u64 counted = 0;
		if (::CopyFileExA(f.src.c_str(), to.c_str(), &CopyProgress, &counted, NULL, 0))
		{
			++copied;
			continue;
		}
		const DWORD code = ::GetLastError();
		if (ERROR_REQUEST_ABORTED == code) { err = "cancelled"; break; }
		s_Feed.Note(NqUtil::Format("! [XMS] build: cannot copy %s (error %u)", f.rel.c_str(), code));
		err = NqUtil::Format("cannot copy %s (error %u)", f.rel.c_str(), code);
		break;
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

// What the file half of a job needs, by value: the main thread works it out and the worker only reads it.
struct SBuildPlan
{
	EditorMod::SManifest	m;
	xr_string				project;
	xr_string				dst;			// <target>\modules\<id>, or <target>\gamedata_<id>
	bool					flat	= false;
	bool					clean	= false;
	xr_string				flat_report;	// its counts come from the editor's own lists
	// a packaging
	bool					package	= false;
	xr_string				stage, out_dir, requires_game;
};

struct SBuildOut
{
	int						files		= 0;
	xr_string				path, err, warnings;
	xr_vector<xr_string>	left_out, stale_names;
	int						stale_count	= 0;
};

static xr_string BytesText(u64 bytes)
{
	if (bytes >= (1ull << 30)) return NqUtil::Format("%.1f GB", double(bytes) / double(1ull << 30));
	if (bytes >= (1ull << 20)) return NqUtil::Format("%.0f MB", double(bytes) / double(1ull << 20));
	return NqUtil::Format("%.0f KB", double(bytes) / 1024.0);
}

// an on-access scanner holds a file it has just seen for a moment, and a folder holding such a file does not move
static bool RenameFolder(LPCSTR from, LPCSTR to)
{
	for (u32 tries = 0; tries < 30; ++tries)
	{
		if (::MoveFileExA(from, to, 0))	return true;
		const DWORD code = ::GetLastError();
		if (ERROR_ACCESS_DENIED != code && ERROR_SHARING_VIOLATION != code)	return false;
		::Sleep(200);
	}
	return false;
}

// a file just deleted lingers while an on-access scanner holds it, and its folder with it
static void RemoveTree(LPCSTR dir)
{
	for (u32 tries = 0; tries < 25 && DirExists(dir); ++tries)
	{
		WipeTree(dir);
		if (::RemoveDirectoryA(dir))	return;
		::Sleep(100);
	}
}

static bool SameFileThere(const SBuildFile& f, const xr_string& folder)
{
	WIN32_FILE_ATTRIBUTE_DATA there;
	return ::GetFileAttributesExA((folder + "\\" + f.rel).c_str(), GetFileExInfoStandard, &there) &&
		   !(there.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) &&
		   ((u64(there.nFileSizeHigh) << 32) | there.nFileSizeLow) == f.size &&
		   0 == ::CompareFileTime(&there.ftLastWriteTime, &f.written);
}

// The build is CLEAN: what ends up in the target is exactly this build and nothing a previous one left. It is written
// beside the last one, in <modules>\.new\<id> (the game reads modules one level deep, so nothing in there is a module),
// and takes its place in one step at the very end: a build that fails or is stopped costs the module nothing, and a game
// started meanwhile never meets half of one.
static bool InstallBuild(const SBuildPlan& plan, SBuildWalk& walk, u64 bytes, SBuildOut& out)
{
	const xr_string dst = plan.dst;
	const bool had = DirExists(dst.c_str());

	// That is destructive when the target is somebody else's module that happens to share the id - so say so, and only
	// replace it once told to.
	if (!plan.clean && had)
	{
		xr_set<xr_string> will;
		for (u32 i = 0; i < walk.files.size(); ++i)
		{
			xr_string key = walk.files[i].rel;
			xr_strlwr(&key[0]);
			will.insert(key);
		}
		out.stale_count = CountStaleRec(dst, xr_string(), will, out.stale_names);
		if (out.stale_count)
		{
			out.err = NqUtil::Format("target already holds a module: %d file(s) there are not part of this build", out.stale_count);
			return false;
		}
	}

	const size_t cut = dst.rfind('\\');
	const xr_string yard = dst.substr(0, cut) + "\\.new";
	const xr_string staged = yard + "\\" + dst.substr(cut + 1);
	const xr_string previous = staged + ".old";
	CreateDirChain(staged.c_str());
	WipeTree(staged.c_str());
	RemoveTree(previous.c_str());
	if (!DirExists(staged.c_str()))	{ out.err = "can't create target folder"; return false; }
	auto clean_up = [&]
	{
		RemoveTree(staged.c_str());
		::RemoveDirectoryA(yard.c_str());
	};

	s_Feed.Stage(kStepCopy, "Comparing with the last build");
	u64 need = 0;
	for (u32 i = 0; i < walk.files.size() && !s_Feed.Cancelled(); ++i)
	{
		SBuildFile& f = walk.files[i];
		f.reuse = had && SameFileThere(f, dst);
		if (!f.reuse) need += f.size;
	}
	ULARGE_INTEGER free_bytes;
	if (::GetDiskFreeSpaceExA(yard.c_str(), &free_bytes, NULL, NULL) && u64(free_bytes.QuadPart) < need)
	{
		out.err = "the build needs " + BytesText(need) + " free on the target's drive and has " + BytesText(u64(free_bytes.QuadPart)) +
			": the new build is written beside the last one before it replaces it";
		clean_up();
		return false;
	}

	s_Feed.Stage(kStepCopy, "Copying files", bytes, (u32)walk.files.size());
	out.files += CopyBuildFiles(walk, staged.c_str(), 0, dst.c_str(), out.err);
	if (!out.err.empty())
	{
		clean_up();
		return false;
	}

	s_Feed.Stage(kStepFinish, "Putting the module in place");
	if (had && !RenameFolder(dst.c_str(), previous.c_str()))
	{
		out.err = "the module in " + dst + " is in use: close the game and build again. It stays as it was";
		clean_up();
		return false;
	}
	if (!RenameFolder(staged.c_str(), dst.c_str()))
	{
		const DWORD code = ::GetLastError();
		if (had) RenameFolder(previous.c_str(), dst.c_str());
		out.err = NqUtil::Format("can't put the build in place (error %u)", code);
		clean_up();
		return false;
	}
	RemoveTree(previous.c_str());
	::RemoveDirectoryA(yard.c_str());
	return true;
}

// The file half of a build, on the job's thread.
static bool WriteBuild(const SBuildPlan& plan, SBuildOut& out)
{
	out.path = plan.dst;
	s_Feed.Stage(kStepCopy, "Listing the files");

	// What ships is "everything the author made", not a fixed list of five folder names: a module may keep any layout it
	// likes and publish it through mod.ltx [vfs]. What stays behind: the editor's own scratch, the sources the game cannot
	// read (rawdata scenes, .object files), tooling (.git and its kind) and whatever the author put in [exclude]. The flat
	// overlay is gamedata alone, filtered exactly like a module build.
	SBuildWalk walk(plan.project.c_str(), plan.m.exclude);
	if (!plan.flat)
		walk.Walk(xr_string());
	else if (EditorMod::ExcludeMatch(plan.m.exclude, "gamedata"))
		walk.left_out.push_back("gamedata");
	else
		walk.Walk("gamedata");
	if (s_Feed.Cancelled()) { out.err = "cancelled"; return false; }
	u64 bytes = 0;
	for (u32 i = 0; i < walk.files.size(); ++i)
		bytes += walk.files[i].size;

	if (plan.flat)
	{
		CreateDirChain(plan.dst.c_str());
		if (!DirExists(plan.dst.c_str())) { out.err = "can't create target folder"; return false; }
		s_Feed.Stage(kStepCopy, "Copying files", bytes, (u32)walk.files.size());
		out.files += CopyBuildFiles(walk, plan.dst.c_str(), 9, nullptr, out.err);	// "gamedata\\" off the front
		if (!out.err.empty()) return false;
		WriteTextFile((plan.dst + "\\_xms_export_report.txt").c_str(), plan.flat_report.c_str());
	}
	else if (!InstallBuild(plan, walk, bytes, out))
		return false;

	if (!walk.left_out.empty())
	{
		xr_string list;
		for (u32 i = 0; i < walk.left_out.size() && i < 12; ++i)
		{
			if (i) list += ", ";
			list += walk.left_out[i];
		}
		if (walk.left_out.size() > 12) list += NqUtil::Format(" and %u more", u32(walk.left_out.size() - 12));
		s_Feed.Note("* [XMS] left out of the build: " + list);
	}
	out.left_out = walk.left_out;

	// The editor takes any name (its file APIs speak UTF-8); the game reads a module
	// through the ANSI APIs of a cp1251 system, where a character cp1251 lacks comes
	// back as '?' and the file never opens. It still ships - it is the author's.
	xr_vector<xr_string> unreadable;
	for (u32 i = 0; i < walk.files.size(); ++i)
	{
		xr_string bad;
		if (!NqUtil::Cp1251Covers(walk.files[i].rel, bad))
			unreadable.push_back(walk.files[i].rel + " (" + bad + ")");
	}
	for (u32 i = 0; i < unreadable.size(); ++i)
		s_Feed.Note("! [XMS] build: the game cannot open " + unreadable[i] + " - a character outside cp1251");
	if (!unreadable.empty())
	{
		if (!out.warnings.empty()) out.warnings += "\n";
		out.warnings += NqUtil::Format("file names the game cannot open (characters outside cp1251): %u", u32(unreadable.size()));
		for (u32 i = 0; i < unreadable.size() && i < 12; ++i)
			out.warnings += "\n  " + unreadable[i];
		if (unreadable.size() > 12) out.warnings += NqUtil::Format("\n  and %u more", u32(unreadable.size() - 12));
	}
	s_Feed.Note(NqUtil::Format("* [XMS] export %s: %d file(s) -> %s", plan.flat ? "flat" : "module", out.files, plan.dst.c_str()));
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
static bool ZipPackage(LPCSTR zip_path, LPCSTR module_dir, xr_vector<SStagedFile>& files,
					   u32 first, u32 last, xr_vector<u8>& buf, SSha256& sha, xr_string& err)
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

		s_Feed.File(f.rel);
		u64 packed = 0;
		while (err.empty())
		{
			DWORD rd = 0;
			if (!::ReadFile(h, buf.data(), (DWORD)buf.size(), &rd, NULL)) { err = "can't read " + f.rel; break; }
			if (!rd) break;
			if (ZIP_OK != zipWriteInFileInZip(zip, buf.data(), rd)) { err = "can't write " + xr_string(zip_path); break; }
			if (!sha.Add(buf.data(), rd)) { err = "can't hash " + f.rel; break; }
			packed	+= rd;
			s_Feed.Add(rd);
			if (s_Feed.Cancelled()) err = "cancelled";
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
		if (s_Feed.Cancelled()) { err = "cancelled"; ok = false; }
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
	return s_Feed.Cancelled() ? PROGRESS_CANCEL : PROGRESS_CONTINUE;
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

// What the thread leaves for the descriptor: the descriptor is signed with the session's key and its card reads the
// project's string tables, so it is written on the main thread, as the job's last step.
struct SReleaseDraft
{
	EditorCatalog::SCardFile	manifest_ref, thumbnail_ref;
	bool						has_thumbnail = false;
};

// stage -> packages -> file index, on the job's thread. `stage` is a private scratch
// folder the caller creates and removes, so a failure leaves `out` untouched.
static bool BuildRelease(const SBuildPlan& plan, EditorMod::SPackageResult& result, SReleaseDraft& draft, SBuildOut& built)
{
	const EditorMod::SManifest& m = plan.m;
	LPCSTR stage = plan.stage.c_str();
	LPCSTR out = plan.out_dir.c_str();
	xr_string& err = built.err;
	xr_string* warnings = &built.warnings;

	// The real module build does the staging: what the release ships is what a build ships.
	if (!WriteBuild(plan, built))
		return false;
	result.left_out = built.left_out;
	const xr_string module_dir = built.path;

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
	s_Feed.Stage(kStepFinish, "Reading the content the way the Mod Browser does");
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

	s_Feed.Stage(kStepFinish, "Packing the release", unpacked, (u32)files.size());
	xr_vector<u8> buf(1 << 20);
	u64 packed = 0;
	for (u32 p = 0; p < parts.size() && err.empty(); ++p)
	{
		EditorMod::SPackageFile pkg;
		pkg.name = m.id + "-" + m.version;
		if (parts.size() > 1) pkg.name += NqUtil::Format(".part%u", p + 1);
		pkg.name += ".zip";
		sprintf_s(path, "%s\\%s", packages_dir, pkg.name.c_str());
		if (!ZipPackage(path, module_dir.c_str(), files, parts[p].first, parts[p].second, buf, sha, err)) break;
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

	s_Feed.Stage(kStepFinish, "Writing the release");
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
		if (!::MoveFileWithProgressA(from, path, &MoveProgress, NULL, MOVEFILE_COPY_ALLOWED | MOVEFILE_REPLACE_EXISTING))
			{ err = s_Feed.Cancelled() ? xr_string("cancelled") : xr_string("can't write ") + path; return false; }
	}
	sprintf_s(path, "%s\\%s", out, result.index.name.c_str());
	if (!WriteTextFile(path, index_text.c_str())) { err = xr_string("can't write ") + path; return false; }

	// The Mod Browser's card pins the manifest and the thumbnail by their hashes (MOD_CATALOG 4, 6.1)
	xr_string thumbnail_path = m.thumbnail;
	for (size_t c = 0; c < thumbnail_path.size(); ++c)
		if (thumbnail_path[c] == '\\') thumbnail_path[c] = '/';
	for (u32 i = 0; i < files.size(); ++i)
	{
		EditorCatalog::SCardFile* ref = files[i].path == "mod.ltx" ? &draft.manifest_ref :
			(!thumbnail_path.empty() && 0 == _stricmp(files[i].path.c_str(), thumbnail_path.c_str())) ? &draft.thumbnail_ref : 0;
		if (!ref) continue;
		ref->path = files[i].path;
		ref->size = files[i].size;
		ref->sha256 = files[i].sha256;
		draft.has_thumbnail |= ref == &draft.thumbnail_ref;
	}
	if (!thumbnail_path.empty() && warnings)
	{
		xr_string bytes;
		string_path picture;
		sprintf_s(picture, "%s\\%s", module_dir.c_str(), m.thumbnail.c_str());
		if (!draft.has_thumbnail)
			*warnings += "the thumbnail " + m.thumbnail + " is not in the build - the Mod Browser shows no picture\n";
		else if (!ReadTextFile(picture, bytes) || !EditorCatalog::ValidThumbnail(bytes.data(), bytes.size()))
		{
			*warnings += "the thumbnail " + m.thumbnail + " is not a 256 x 256 DXT1/DXT5 DDS - the Mod Browser shows no picture\n";
			draft.has_thumbnail = false;
		}
	}

	result.out_dir			= out;
	result.requires_game	= plan.requires_game;
	result.files			= (int)files.size();
	result.unpacked			= unpacked;
	result.packed			= packed;
	return true;
}

// The descriptor, on the main thread: it goes last, because it is the file that vouches for the other two kinds.
static bool FinishRelease(const SBuildPlan& plan, const SReleaseDraft& draft, EditorMod::SPackageResult& result, xr_string& err, xr_string& warnings)
{
	const EditorMod::SManifest& m = plan.m;
	xr_string text = "; XMS module release descriptor. Generated by Grizzly Editor.\r\n[release]\r\n";
	AppendKey(text, "schema", "1", 13);
	AppendKey(text, "id", m.id.c_str(), 13);
	AppendKey(text, "version", m.version.c_str(), 13);
	if (!plan.requires_game.empty())
		AppendKey(text, "requires_game", plan.requires_game.c_str(), 13);
	AppendKey(text, "files", NqUtil::Format("%d", result.files).c_str(), 13);
	AppendKey(text, "unpacked", NqUtil::Format("%llu", result.unpacked).c_str(), 13);
	AppendKey(text, "index", (result.index.name + NqUtil::Format(", %llu, ", result.index.size) + result.index.sha256).c_str(), 13);
	text += "\r\n[packages]\r\n";
	for (u32 p = 0; p < result.packages.size(); ++p)
		text += result.packages[p].name + NqUtil::Format(" = %llu, ", result.packages[p].size) + result.packages[p].sha256 + "\r\n";
	text += EditorCatalog::CardSection(plan.project.c_str(), m, draft.manifest_ref, draft.has_thumbnail ? &draft.thumbnail_ref : 0);

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
	else
		warnings += key.exists ? (key.locked ? "the release is not signed: the author key is locked - give its password first\n" :
											  "the release is not signed: " + key.error + "\n") :
			xr_string("the release is not signed - the Mod Browser takes signed releases only (Mod > Submit to Mod Browser sets up the author key)\n");

	string_path path;
	sprintf_s(path, "%s\\%s.update.ltx", plan.out_dir.c_str(), m.id.c_str());
	if (!WriteTextFile(path, text.c_str())) { err = xr_string("can't write ") + path; return false; }
	// a publisher checks every uploaded asset against these, the descriptor included
	SSha256 sha;
	result.descriptor_file.name = m.id + ".update.ltx";
	result.descriptor_file.size = text.size();
	if (!sha.Add(text.data(), (u32)text.size()) || !sha.Finish(result.descriptor_file.sha256))
		{ err = "can't hash the release descriptor"; return false; }
	result.descriptor = path;
	Msg("* [XMS] release %s %s: %d file(s), %llu byte(s) in %u package(s) -> %s",
		m.id.c_str(), m.version.c_str(), result.files, result.packed, (u32)result.packages.size(), plan.out_dir.c_str());
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

// temp\ is editor-only scratch that no export ever reads, so the stage can sit
// inside the project; the pid keeps two editors on one project apart
static bool PackageFiles(const SBuildPlan& plan, EditorMod::SPackageResult& result, SReleaseDraft& draft, SBuildOut& built)
{
	RemoveDeadStages((plan.project + "\\temp").c_str());
	CreateDirChain(plan.stage.c_str());
	if (!DirExists(plan.stage.c_str())) { built.err = "can't create the staging folder in the project's temp"; return false; }
	WipeTree(plan.stage.c_str());
	const bool ok = BuildRelease(plan, result, draft, built);
	RemoveTree(plan.stage.c_str());
	if (!ok) result = EditorMod::SPackageResult();
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
using ModToken = GrizzlyTheme::ColorToken;

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
	GrizzlyMCP::ReleaseTexture(s_ThumbTexture);
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
	char	updates_off[4];			// "1": [module] disable_game_updates
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
	{ offsetof(SSettingsForm, updates_off),	sizeof(SSettingsForm::updates_off) },
	{ offsetof(SSettingsForm, tags),		sizeof(SSettingsForm::tags) },
	{ offsetof(SSettingsForm, pda),			sizeof(SSettingsForm::pda) },
};

enum ESettingsPage { pgGeneral = 0, pgBrowser, pgMode, pgRelations, pgContents, pgPda, pgCommands, pgCount };
struct SSettingsPage { LPCSTR id; LPCSTR label; XUI::Icon icon; LPCSTR about; };
static const SSettingsPage kSettingsPages[pgCount] =
{
	{ "general",		"General",			XUI::Icon::Info,			"What the game's Mods menu shows, and where the game looks for updates." },
	{ "mod_browser",	"Mod Browser",		XUI::Icon::Globe,			"Shown next to the module in the game's Mods window and Mod Browser. Listing it there: Mod > Submit to Mod Browser." },
	{ "game_mode",		"Game Mode",		XUI::Icon::Mode,			"A module targets one mode: the plain game, a mode the linked game has, or a new mode it brings. A mode is not the mod: the mod's name is on General." },
	{ "relations",		"Relations",		XUI::Icon::Relations,		"The modules this one needs, loads around or cannot run with. Only matters when the player runs several modules at once: a standalone mod leaves all four empty." },
	{ "build_contents",	"Build Contents",	XUI::Icon::Exclude,			"The project folder is your working place as well as the module. The build ships everything in it except what is listed here." },
	{ "pda",			"PDA",				XUI::Icon::PdaInterface,	"A module can give the PDA screens of its own. The layout named here is the one the game shows." },
	{ "commands",		"Commands",			XUI::Icon::Terminal,		"Console commands the module adds. The player types the name; the game calls the function of the project's scripts with what follows it and prints what it returns." },
};

struct SSettingsProblem
{
	int			page;
	xr_string	field;		// the row it is shown under ("commands.<n>.<part>" for a command's)
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

// the Commands page: the list as edited and as loaded (the form's fixed fields cannot hold a list),
// and what its checks read - taken when the page is picked, never per frame
static xr_vector<EditorMod::SCommand>	s_Commands;
static xr_vector<EditorMod::SCommand>	s_CommandsBase;
static xr_vector<xr_string>				s_CmdFunctions;		// <script>.<function> of the project's scripts
static xr_vector<std::pair<xr_string, xr_string> >	s_CmdTaken;	// {command, module} of the linked game's other modules
static char								s_FunctionSearch[64] = {};

static bool SameCommands(const xr_vector<EditorMod::SCommand>& a, const xr_vector<EditorMod::SCommand>& b)
{
	if (a.size() != b.size()) return false;
	for (u32 i = 0; i < a.size(); ++i)
		if (a[i].name != b[i].name || a[i].function != b[i].function || a[i].help != b[i].help || a[i].args != b[i].args ||
			a[i].cheat != b[i].cheat || a[i].declared != b[i].declared || a[i].extra != b[i].extra)
			return false;
	return true;
}

static void RefreshCommandData()
{
	EditorMod::ScriptFunctions(EditorProject::Root(), s_CmdFunctions);
	EditorMod::InstalledCommands(s_Form.id, s_CmdTaken);
}

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
	strncpy_s(f.updates_off, m.disable_game_updates ? "1" : "", _TRUNCATE);
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
	m.disable_game_updates = f.updates_off[0] != 0;
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
	return !SameCommands(s_Commands, s_CommandsBase);
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
	s_CommandsBase = m.commands;
	s_Commands = s_CommandsBase;
	s_SettingsThumb = m.thumbnail;
	s_FormGen = EditorMod::ManifestGeneration();
	s_FormRoot = EditorProject::Root();
	s_SettingsStatus.clear();
	ThumbnailCleanup();
	RefreshCommandData();

	string_path probe;
	sprintf_s(probe, "%s\\gamedata", EditorProject::Root());
	s_GatedFiles = CountFilesRec(probe, 0);
	sprintf_s(probe, "%s\\patch", EditorProject::Root());
	s_GatedPatches = CountFilesRec(probe, "_readme.txt");
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
	// the list is one field: untouched, it follows the file
	if (SameCommands(s_Commands, s_CommandsBase))
		s_Commands = m.commands;
	s_CommandsBase = m.commands;
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

// Under each command's rows: what the game would refuse (errors, Save waits) and what the build or the linked
// game's other modules will say (warnings) - the sentences of ValidateCommands' codes.
static void CheckCommands(xr_vector<SSettingsProblem>& out)
{
	auto add = [&out](u32 i, LPCSTR part, XUI::Tone tone, bool blocks, const xr_string& text)
	{
		SSettingsProblem& p = out.emplace_back();
		p.page = pgCommands;
		p.field = NqUtil::Format("commands.%u.%s", i, part);
		p.tone = tone;
		p.blocks = blocks;
		p.text = text;
	};
	for (u32 i = 0; i < s_Commands.size(); ++i)
	{
		const EditorMod::SCommand& c = s_Commands[i];
		if (c.name.empty())
			add(i, "name", XUI::Tone::Required, true, "Required: what the player types in the console.");
		else if (!EditorMod::ValidCommandName(c.name.c_str()))
			add(i, "name", XUI::Tone::Error, true, "Use a-z, 0-9 and _ only: the console finds a command by its exact name.");
		else if (StockCommandWord(c.name.c_str()))
			add(i, "name", XUI::Tone::Error, true, "The game has a command of this name: it keeps its own and refuses this one.");
		else if (LPCSTR family = StockCommandFamily(c.name.c_str()))
			add(i, "name", XUI::Tone::Warning, false,
				xr_string("Named like the game's own ") + family + "* commands: if the game has one of this name, it keeps it.");
		for (u32 k = 0; k < i; ++k)
			if (!c.name.empty() && 0 == _stricmp(s_Commands[k].name.c_str(), c.name.c_str()))
			{
				add(i, "name", XUI::Tone::Error, true, "Declared twice: the game takes the first and refuses this one.");
				break;
			}
		for (const std::pair<xr_string, xr_string>& t : s_CmdTaken)
			if (0 == _stricmp(t.first.c_str(), c.name.c_str()))
			{
				add(i, "name", XUI::Tone::Warning, false, "Module " + t.second + " of the linked game has this command too: "
					"the one that loads first keeps it.");
				break;
			}
		if (c.function.empty())
			add(i, "function", XUI::Tone::Required, true, "Required: <script>.<function> of the project's scripts.");
		else if (!EditorMod::ValidCommandFunction(c.function.c_str()))
			add(i, "function", XUI::Tone::Error, true, "<script>.<function>, e.g. my_mod.status.");
		else if (!HasScriptFunction(s_CmdFunctions, c.function))
			add(i, "function", XUI::Tone::Warning, false, "gamedata\\scripts has no such function: the build refuses the module until it does.");
		xr_string bad;
		if (!NqUtil::Cp1251Covers(c.help, bad))
			add(i, "help", XUI::Tone::Warning, false, "Outside cp1251 (" + bad + "): the console shows '?' for it.");
		if (!NqUtil::Cp1251Covers(c.args, bad))
			add(i, "args", XUI::Tone::Warning, false, "Outside cp1251 (" + bad + "): the console shows '?' for it.");
	}
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
	CheckCommands(out);
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
		if (p.field == field && p.tone == XUI::Tone::Error)
			return flags | XUI::PropInvalid;
	return flags;
}

static void RowMessages(LPCSTR field)
{
	for (const SSettingsProblem& p : s_SettingsProblems)
		if (p.field == field)
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
	// untouched, the list stays what mod.ltx says now - a save elsewhere or an edit by hand
	if (!SameCommands(s_Commands, s_CommandsBase))
		m.commands = s_Commands;
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
	s_CommandsBase = m.commands;
	s_Commands = s_CommandsBase;
	s_SettingsThumb = m.thumbnail;
	s_FormGen = EditorMod::ManifestGeneration();
	s_SettingsStatus = thumb_err.empty() ? xr_string() : "The picture was not saved: " + thumb_err;
	return true;
}

static void RevertSettings()
{
	s_Form = s_FormBase;
	s_Commands = s_CommandsBase;
	if (!s_ThumbPending.empty()) ::DeleteFileA(s_ThumbPending.c_str());
	s_ThumbPending.clear();
	s_ThumbRemove = false;
	s_ThumbError.clear();
	s_SettingsStatus.clear();
}

// ---- the pages ------------------------------------------------------------------

// A last resort, so it asks: turning it on names everything the players lose. Turning it off needs no question.
static bool s_AskGameUpdates = false;

static void AskGameUpdates()
{
	XUI::ConfirmSpec spec;
	spec.id = "disable_game_updates";
	spec.title = "Disable Game Updates";
	spec.headline = "Everyone who installs this module stops getting updates of the game. Disable them only as a last resort.";
	spec.items.push_back("The game no longer checks for updates, so its fixes never reach these players.");
	spec.items.push_back("Send Bug Report disappears from the main menu.");
	spec.items.push_back("The menu says in red that updates were disabled, and names this module.");
	spec.items.push_back("It lasts for as long as the module is installed, and other modules cannot turn it back on.");
	spec.consequence = "Right only for a module an update would break. If a game update merely might conflict with yours, keep updates on and release a fix instead.";
	spec.action = "Disable Updates";
	spec.destructive = true;
	XUI::Confirm(spec, [](XUI::Answer answer) { if (answer == XUI::Answer::Yes) strncpy_s(s_Form.updates_off, "1", _TRUNCATE); });
}

static void DrawGameUpdatesRow()
{
	SSettingsForm& f = s_Form;
	bool off = f.updates_off[0] != 0;
	if (s_AskGameUpdates)
	{
		s_AskGameUpdates = false;
		AskGameUpdates();
	}
	if (!XUI::PropertyRow("Disable Game Updates", "Only for a module that replaces the game's own files so deeply that an update would break it. "
		"Saved with the module's other settings, and applied by the build.", XUI::PropInline))
		return;
	if (ImGui::Checkbox("##updates_off", &off))
	{
		if (!off)
			f.updates_off[0] = 0;
		else
			AskGameUpdates();
	}
	if (f.updates_off[0])
		XUI::PropertyMessage(XUI::Tone::Warning, "Players of this module get no game updates and cannot send bug reports.");
}

static void DrawGeneralPage()
{
	SSettingsForm& f = s_Form;
	if (XUI::BeginPropertyGrid("mod_settings", .28f))
	{
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
		bool id_problem = false;
		for (const SSettingsProblem& problem : s_SettingsProblems)
			id_problem |= problem.field == "id";
		if (XUI::PropertySection("More##general", id_problem))
		{
			if (XUI::PropertyRow("Module ID", "Technical, never shown to the player: the folder the module installs into (<game>\\modules\\<id>) "
				"and the name other modules require it by. Seeded from the project folder - change it before the first release.",
				RowFlags("id", XUI::PropRequired)))
			{
				ImGui::InputText("##id", f.id, sizeof(f.id));
				RowMessages("id");
			}
			DrawGameUpdatesRow();
			XUI::EndPropertySection();
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
		GrizzlyMCP::ReleaseTexture(s_ThumbTexture);
		s_ThumbTexture = shown.empty() ? 0 : DX11TextureFromFile(shown.c_str());
		s_ThumbTextureOf = shown;
	}

	const ImVec2 box(XUI::Pt(96.f), XUI::Pt(96.f));
	const ImVec2 at = ImGui::GetCursorScreenPos();
	ImDrawList* dl = ImGui::GetWindowDrawList();
	dl->AddRectFilled(at, ImVec2(at.x + box.x, at.y + box.y), GrizzlyTheme::U32(ModToken::Inset), XUI::Pt(4.f));
	if (s_ThumbTexture)
		ImGui::Image((ImTextureID)s_ThumbTexture, box);
	else
	{
		// the frame of the picture, so the row does not jump
		dl->AddRect(at, ImVec2(at.x + box.x, at.y + box.y), GrizzlyTheme::U32(ModToken::BorderSubtle), XUI::Pt(4.f));
		XUI::DrawIcon(dl, XUI::Icon::Picture, ImVec2(at.x + box.x * .5f - XUI::Pt(16.f), at.y + box.y * .5f - XUI::Pt(16.f)), 32.f,
			GrizzlyTheme::U32(ModToken::TextMuted));
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
				EditorGameModes::Invalidate();
			if (modes.empty())
				XUI::PropertyMessage(XUI::Tone::Warning, scan_err.empty() ? "The linked game declares no modes, only options." : scan_err.c_str());
			RowMessages("mode");
		}
	}
	else if (kind == 2)
	{
		if (XUI::PropertyRow("Mode ID", "Technical id of the MODE, not of the mod: the module's level work is gated on it and the "
			"new-game screen offers it. It may match the module id or not.", RowFlags("pmode_id", XUI::PropRequired)))
		{
			ImGui::InputText("##pmode_id", f.pmode_id, sizeof(f.pmode_id));
			RowMessages("pmode_id");
		}
		if (XUI::PropertyRow("Mode Title", "What the new-game screen lists the mode as. It names the mode, not the mod: "
			"the Mods menu shows the Name on General instead."))
		{
			ImGui::InputText("##pmode_title", f.pmode_title, sizeof(f.pmode_title));
			RowMessages("pmode_title");
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

// A function of the project's scripts, picked from a searchable list.
static bool FunctionPicker(LPCSTR popup, xr_string& picked)
{
	if (!ImGui::BeginPopup(popup)) return false;
	bool done = false;
	const float width = XUI::Em(24.f);
	if (ImGui::IsWindowAppearing())
	{
		s_FunctionSearch[0] = 0;
		ImGui::SetKeyboardFocusHere();
	}
	ImGui::SetNextItemWidth(width);
	XUI::SearchField("##search", s_FunctionSearch, sizeof(s_FunctionSearch), "Search the project's functions");
	xr_vector<const xr_string*> rows;
	for (const xr_string& fn : s_CmdFunctions)
		if (XUI::Match(s_FunctionSearch, fn.c_str()))
			rows.push_back(&fn);
	// Enter takes the first row
	if (!rows.empty() && ImGui::IsKeyPressed(ImGuiKey_Enter, false) && ImGui::IsWindowFocused(ImGuiFocusedFlags_ChildWindows))
	{
		picked = *rows[0];
		done = true;
		ImGui::CloseCurrentPopup();
	}
	ImGui::BeginChild("##functions", ImVec2(width, XUI::Pt(240.f)));
	for (u32 i = 0; i < rows.size() && !done; ++i)
		if (ImGui::Selectable(rows[i]->c_str()))
		{
			picked = *rows[i];
			done = true;
			ImGui::CloseCurrentPopup();
		}
	if (rows.empty())
		XUI::StatusText(XUI::Tone::Muted, s_CmdFunctions.empty() ? "The project's gamedata\\scripts has no function yet: write one, then pick it here."
			: "No function matches.");
	ImGui::EndChild();
	ImGui::EndPopup();
	return done;
}

// One section per command, the same label-left grid as the other pages; Add Command under them.
static void DrawCommandsPage()
{
	if (XUI::BeginPropertyGrid("mod_settings", .28f))
	{
		int remove = -1;
		for (u32 i = 0; i < s_Commands.size(); ++i)
		{
			EditorMod::SCommand& c = s_Commands[i];
			ImGui::PushID((int)i);
			const xr_string field = NqUtil::Format("commands.%u.", i);
			const xr_string f_name = field + "name", f_function = field + "function", f_help = field + "help", f_args = field + "args";
			const xr_string title = c.name.empty() ? xr_string("New Command") : c.name;
			if (XUI::PropertySection(title.c_str(), true, c.cheat ? "cheat" : nullptr))
			{
				if (XUI::PropertyRow("Name", "What the player types in the console: a-z, 0-9 and _. A name the game has, or that a module "
					"loading earlier took, is refused by the game.", RowFlags(f_name.c_str(), XUI::PropRequired)))
				{
					char name[72];
					strncpy_s(name, c.name.c_str(), _TRUNCATE);
					if (ImGui::InputTextWithHint("##name", "my_mod_status", name, sizeof(name)))
						c.name = name;
					if (XUI::PropertyTrailing("##remove", XUI::Icon::Delete, "Take the command out"))
						remove = (int)i;
					RowMessages(f_name.c_str());
				}
				if (XUI::PropertyRow("Function", "<script>.<function> of the project's gamedata\\scripts. The game calls it with the text "
					"typed after the name, the module id and the command's name, and prints the string it returns.",
					RowFlags(f_function.c_str(), XUI::PropRequired)))
				{
					char function[192];
					strncpy_s(function, c.function.c_str(), _TRUNCATE);
					if (ImGui::InputTextWithHint("##function", "my_mod.status", function, sizeof(function)))
					{
						c.function = function;
						c.declared = true;
					}
					const ImVec2 below(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y + XUI::Pt(2.f));
					if (XUI::PropertyTrailing("##pick", XUI::Icon::ChevronDown, "Pick a function of the project's scripts"))
						XUI::Shell::OpenPopupAt("pick_function", below);
					if (XUI::PropertyTrailing("##rescan", XUI::Icon::Refresh, "Read the project's scripts again"))
						RefreshCommandData();
					xr_string picked;
					if (FunctionPicker("pick_function", picked))
					{
						c.function = picked;
						c.declared = true;
					}
					RowMessages(f_function.c_str());
				}
				if (XUI::PropertyRow("Help", "The line the console's help shows for it. Plain text or a string table id.",
					RowFlags(f_help.c_str())))
				{
					char help[256];
					strncpy_s(help, c.help.c_str(), _TRUNCATE);
					if (ImGui::InputText("##help", help, sizeof(help)))
						c.help = help;
					RowMessages(f_help.c_str());
				}
				if (XUI::PropertyRow("Arguments", "What it takes after the name, e.g. <night 1-7>: shown by help and as the hint while "
					"typing. The function reads them from its first argument (xms.command_args splits them into words).",
					RowFlags(f_args.c_str())))
				{
					char args[128];
					strncpy_s(args, c.args.c_str(), _TRUNCATE);
					if (ImGui::InputTextWithHint("##args", "none", args, sizeof(args)))
						c.args = args;
					RowMessages(f_args.c_str());
				}
				if (XUI::PropertyRow("Cheat", "Runs only in Dead Air's debug mode (the game started with -dbg); otherwise the console "
					"says so and nothing runs."))
					ImGui::Checkbox("##cheat", &c.cheat);
				XUI::EndPropertySection();
			}
			ImGui::PopID();
		}
		XUI::EndPropertyGrid();
		if (remove >= 0)
			s_Commands.erase(s_Commands.begin() + remove);
	}
	if (s_Commands.empty())
		XUI::Prose("No commands. A command is a line of [commands] in mod.ltx: the name the player types and the function it calls.");
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
	ImGui::SetCursorPosX(ImGui::GetCursorPosX() + XUI::Pt(6.f));
	if (XUI::Button("Add Command"))
		s_Commands.push_back(EditorMod::SCommand());
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
				// a function written since: the scripts are read when the page is picked
				if (i == pgCommands) RefreshCommandData();
			}
		}
	}
	XUI::EndList();
	ImGui::SameLine(0.f, XUI::Pt(8.f));

	// the form: one column of up to 44 em beside the nav, the scrollbar at the right edge of the tab
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(16.f), XUI::Pt(8.f)));
	// layout children: the document's own fill, not the inset of a list
	ImGui::PushStyleColor(ImGuiCol_ChildBg, GrizzlyTheme::U32(ModToken::Base));
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
		case pgCommands:	DrawCommandsPage();	break;
		}
		ImGui::EndChild();
	}
	ImGui::EndChild();

	// the sticky footer: what stops the save, if anything does
	const ImVec2 at = ImGui::GetCursorScreenPos();
	const float right = at.x + ImGui::GetContentRegionAvail().x;
	ImGui::GetWindowDrawList()->AddLine(ImVec2(at.x, at.y), ImVec2(right, at.y),
		GrizzlyTheme::U32(ModToken::BorderSubtle));
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
			tb.Status(why.c_str(), XUI::Tone::Error);
		else if (!s_SettingsStatus.empty())
			tb.Status(s_SettingsStatus.c_str(), XUI::Tone::Error);
		else
			tb.Status("Saved to mod.ltx as you edit", XUI::Tone::Muted);
	}
	// a field still being typed in is not an edit yet
	if (dirty && !blocking && !ImGui::IsAnyItemActive())
	{
		xr_string err;
		SaveSettings(err);
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

//------------------------------------------------------------------------------
// the job
//------------------------------------------------------------------------------
enum EJobPhase	{ jpIdle, jpChecks, jpFiles };
enum ECheck		{ ckBake, ckManifest, ckQuestsBegin, ckQuests, ckQuestsEnd, ckWorld, ckCommands, ckPda, ckScreens, ckModes, ckTarget, ckDone };

static const char* const kCheckStage[] =
{
	"Baking the scene", "Reading the manifest", "Checking the quests", "Checking the quests", "Checking the quests", "Checking the world rules",
	"Checking the console commands", "Checking the PDA", "Checking the screens and music", "Checking the characters", "Checking the target folder",
	"Starting",
};

static const u32 kCheckSliceMs	= 12;
static const u32 kCheckSlowMs	= 100;
static const u32 kFramesFirst	= 2;	// the row is on the screen before the first check

struct SJob
{
	EJobPhase					phase		= jpIdle;
	EditorMod::SJobRequest		rq;
	xr_string					root;
	SBuildPlan					plan;
	xr_string					bake_note, warnings;
	u32							check		= ckBake;
	u32							started		= 0;
	u32							frames		= 0;
	NqExport::SProjectCheck*	quests		= nullptr;
};

static SJob							s_Job;
static std::thread					s_JobThread;
// what the thread answers; read once s_JobDone is up
static std::atomic<bool>			s_JobDone	{ false };
static std::atomic<bool>			s_JobWarmed	{ false };
static bool							s_JobOk		= false;
static SBuildOut					s_JobOut;
static EditorMod::SPackageResult	s_JobPackage;
static SReleaseDraft				s_JobDraft;
static EditorMod::SJobResult		s_JobLast;

// The checks walk the project on the main thread, and a walk costs a round trip per folder while the folders are not in
// the file cache: seconds on a big mod. Walked here first, they are.
static void TouchTree(const xr_string& dir)
{
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA((dir + "\\*").c_str(), &fd);
	if (INVALID_HANDLE_VALUE == h)	return;
	do
	{
		if ((fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) && !(fd.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) &&
			!EditorProject::IsToolingEntry(fd.cFileName) && strcmp(fd.cFileName, ".") && strcmp(fd.cFileName, ".."))
			TouchTree(dir + "\\" + fd.cFileName);
	} while (!s_Feed.Cancelled() && ::FindNextFileA(h, &fd));
	::FindClose(h);
}

static void WarmThread(xr_string root)
{
	TouchTree(root);
	s_Feed.Stage(kStepCheck, kCheckStage[ckBake]);
	s_JobWarmed = true;
}

static void JobThread()
{
	const SBuildPlan& plan = s_Job.plan;
	s_JobOk = plan.package ? PackageFiles(plan, s_JobPackage, s_JobDraft, s_JobOut) : WriteBuild(plan, s_JobOut);
	s_JobDone = true;
}

static void DrainJobLog()
{
	xr_vector<xr_string> lines;
	{
		std::lock_guard<std::mutex> guard(s_Feed.lock);
		lines.swap(s_Feed.log);
	}
	for (const xr_string& line : lines)
		Msg("%s", line.c_str());
}

static void AddWarning(xr_string& to, const xr_string& line)
{
	if (!to.empty()) to += "\n";
	to += line;
}

// the lines of a validator: E... refuses the build, the others are warnings
static xr_string SortProblems(const xr_vector<xr_string>& problems, xr_string& warnings)
{
	xr_string errors;
	for (const xr_string& p : problems)
		AddWarning(p[0] == 'E' ? errors : warnings, p);
	return errors;
}

// The report a flat export leaves in its folder: XMS-only content is not representable as a plain gamedata overlay.
static xr_string FlatReport(LPCSTR project_root, const EditorMod::SManifest& m)
{
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
	xr_vector<xr_string> characters;
	EditorCharacters::List(project_root, characters);
	if (!c_patch && !c_spawn && !c_scripts && !c_levels && !c_quests && m.mode.empty() && m.provides_modes.empty() && characters.empty())
		return rep + "fully vanilla-compatible\r\n";
	// the game reads a mode and a character of a module, and a flat overlay is none
	if (!characters.empty())	{ sprintf_s(line, "characters: %d file(s) - NOT exported (XMS-only)\r\n", int(characters.size())); rep += line; }
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
	return rep;
}

// Where the build goes, and whether it may. A build replaces what is there, so a target that is a link - one made so the
// game runs the working copy directly - or that is, holds or sits in the project would take the author's sources with it.
// Judged on where the target would be, before a single folder of it is created.
static bool CheckTarget(SJob& job, xr_string& err)
{
	SBuildPlan& plan = job.plan;
	const EditorMod::SManifest& m = plan.m;
	LPCSTR project_root = job.root.c_str();
	plan.project	= job.root;
	plan.flat		= EditorMod::jkExportFlat == job.rq.kind;
	plan.package	= EditorMod::jkPackage == job.rq.kind;
	plan.clean		= job.rq.clean || plan.package;
	if (plan.package)
		plan.stage = NqUtil::Format("%s\\temp\\release_stage_%u", project_root, (u32)::GetCurrentProcessId());

	char target[MAX_PATH] = {};
	NormalizePath(plan.package ? plan.stage.c_str() : job.rq.target.c_str(), target, sizeof(target));
	if (!target[0])							{ err = "target folder not set"; return false; }
	if (xr_strlen(target) + m.id.size() + 32 >= MAX_PATH)
											{ err = "target path too long"; return false; }

	// A module goes to <game>\modules\, NOT to mods\: that one is the folder
	// JSGME manages, and a module sitting there is a module JSGME offers to
	// copy over the game - the merge a module exists to avoid. A flat overlay
	// is the opposite case: that IS a JSGME mod, so it keeps its own name.
	string_path dst;
	if (plan.flat)	sprintf_s(dst, "%s\\gamedata_%s", target, m.id.c_str());
	else			sprintf_s(dst, "%s\\modules\\%s", target, m.id.c_str());
	if (InsideGameGamedata(dst) || InsideGameGamedata(target))
	{
		err = "target is inside the game's gamedata - a mod must stay in its own folder "
			  "(export to the game ROOT: the module goes to <game>\\modules\\<id>)";
		return false;
	}
	plan.dst = dst;

	xr_string real_dst, real_src;
	const bool known = ProjectedRealPath(dst, real_dst) && RealFolder(project_root, real_src);
	const DWORD attr = ::GetFileAttributesA(dst);
	if (INVALID_FILE_ATTRIBUTES != attr && (attr & FILE_ATTRIBUTE_REPARSE_POINT))
	{
		err = "the target folder is a link";
		if (known) { err += " to "; err += real_dst; }
		err += " - a build replaces it, and would replace what it points at. Remove the link, or build to another folder";
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
		return false;
	}
	if (plan.flat) plan.flat_report = FlatReport(project_root, m);
	return true;
}

// One check, on the main thread; false = the job ends here and `err` says why. What refuses a build is here, where the
// author can still fix it: quest and behaviour graphs ship as they are, so a broken one would only be found by the player.
static bool RunCheck(SJob& job, xr_string& err)
{
	SBuildPlan& plan = job.plan;
	EditorMod::SManifest& m = plan.m;
	LPCSTR root = job.root.c_str();
	switch (job.check)
	{
	case ckBake:
		EditorMod::RunPreBuildBake(job.bake_note);
		break;
	case ckManifest:
		if (!EditorMod::Load(root, m))			{ err = "mod.ltx not found in the project"; return false; }
		if (EditorMod::jkPackage == job.rq.kind)
		{
			if (!ReleaseGate(m, err))			return false;
			if (!job.rq.requires_game.empty() && !DottedNumber(job.rq.requires_game.c_str(), 3, 3))
				{ err = "invalid requires_game: MAJOR.MINOR.PATCH expected, e.g. 1.4.1"; return false; }
			string_path out;
			if (!ResolveReleaseDir(root, job.rq.out_dir.c_str(), m.version, out, err)) return false;
			plan.out_dir = out;
			plan.requires_game = job.rq.requires_game;
		}
		else if (!EditorMod::ValidateId(m.id.c_str()))
			{ err = "module id missing or invalid in mod.ltx (allowed: a-z 0-9 _ . -)"; return false; }
		// A finished module always states which game it is for. "Nothing chosen"
		// is not a third option - it is how content ends up applied in a campaign
		// it was never built for.
		if (m.mode.empty() && m.provides_modes.empty() && 0 != _stricmp(m.target.c_str(), "default"))
		{
			err = "no target game mode: open Mod > Mod Settings, Game Mode, and pick one "
				  "(the ordinary game, a mode of the linked game, or a new mode this module adds)";
			return false;
		}
		break;
	case ckQuestsBegin:
		job.quests = NqExport::BeginProjectCheck();
		break;
	case ckQuests:
		if (!NqExport::StepProjectCheck(job.quests, kCheckSliceMs))
			return true;
		break;
	case ckQuestsEnd:
	{
		xr_vector<xr_string> nq_log;
		int nq_errors = 0, nq_warnings = 0;
		const bool nq_ok = NqExport::EndProjectCheck(job.quests, err, nq_log, &nq_errors, &nq_warnings);
		job.quests = nullptr;
		if (nq_warnings)
		{
			// the lines the author needs to read, not a count they have to go looking for
			job.warnings = NqUtil::Format("quest warnings: %d", nq_warnings);
			for (u32 i = 0; i < nq_log.size(); ++i)
			{
				if (nq_log[i].find(": E") != xr_string::npos) continue;
				job.warnings += "\n  ";
				job.warnings += nq_log[i];
			}
		}
		if (!nq_ok) return false;
		break;
	}
	case ckWorld:
	{
		// a second Player Start or a fourth new-game choice is a decision the game would make for the author
		// (the first start wins, the fourth row never shows) - refused; a level or transition the game lacks is a warning
		xr_vector<xr_string> problems;
		EditorWorld::Validate(root, m, problems);
		err = SortProblems(problems, job.warnings);
		if (!err.empty()) return false;
		break;
	}
	case ckCommands:
	{
		// the game refuses a bad name, a taken one or a function it cannot call with a log line the player reads
		xr_vector<xr_string> problems;
		EditorMod::ValidateCommands(root, m, problems);
		err = SortProblems(problems, job.warnings);
		if (!err.empty()) return false;
		break;
	}
	case ckPda:
	{
		// the game refuses a layout with an error and keeps its stock PDA
		xr_vector<xr_string> pda_errors, pda_warnings;
		EditorPda::ValidateForBuild(root, pda_errors, pda_warnings);
		for (const xr_string& p : pda_warnings) AddWarning(job.warnings, p);
		if (!pda_errors.empty())
		{
			err = "The PDA layout has errors:";
			for (const xr_string& p : pda_errors) { err += "\n"; err += p; }
			return false;
		}
		break;
	}
	case ckScreens:
	{
		// screens and music name files of the project: one the module would not hold is a screen the game cannot show
		xr_vector<xr_string> problems;
		EditorScreens::Validate(root, m, problems);
		const xr_string refused = SortProblems(problems, job.warnings);
		if (!refused.empty())
		{
			err = "The screens and music have errors:\n" + refused;
			return false;
		}
		break;
	}
	case ckModes:
		if (m.disable_game_updates)
			AddWarning(job.warnings, "This module turns off the game's updates and bug reports for everyone who has it.");
		for (u32 i = 0; i < m.provides_modes.size(); ++i)
		{
			const EditorMod::SProvidesMode& mode = m.provides_modes[i];
			if (mode.id.empty()) continue;
			if (!EditorMod::ValidateModeId(mode.id.c_str()) || mode.id.size() > 64)
			{
				err = "invalid declared mode id '" + mode.id + "' (allowed: a-z 0-9 _ . -, up to 64 chars)";
				return false;
			}
			// the game shows the title in cp1251, and an export driven from MCP never sees the field's own warning
			xr_string outside;
			if (!NqUtil::Cp1251Covers(mode.title, outside))
				AddWarning(job.warnings, "Mode '" + mode.id + "': the title has characters outside cp1251 (" + outside + "), the player will see a look-alike or '?'.");
		}
		// the manifest and the assets are the whole description and the game makes the rest of them
		if (!EditorCharacters::CheckForBuild(root, err)) return false;
		break;
	case ckTarget:
		if (!CheckTarget(job, err)) return false;
		break;
	}
	++job.check;
	s_Feed.Stage(kStepCheck, kCheckStage[job.check]);
	return true;
}

static void EndJob(EditorMod::SJobResult& result)
{
	if (s_JobThread.joinable()) s_JobThread.join();
	DrainJobLog();
	result.bake_note = s_Job.bake_note;
	if (s_Job.quests) NqExport::DropProjectCheck(s_Job.quests);
	const auto done = s_Job.rq.done;
	s_Job = SJob();
	s_JobLast = result;
	if (done) done(s_JobLast);
}

// a job without a thread: refused or stopped before the files
static void EndChecks(bool cancelled, const xr_string& err)
{
	EditorMod::SJobResult result;
	result.cancelled = cancelled;
	result.err = cancelled ? xr_string("cancelled") : err;
	result.warnings = s_Job.warnings;
	EndJob(result);
}

static void StepJobChecks()
{
	// a job of a project that has been closed meanwhile has nothing left to check
	if (s_Feed.Cancelled() || !EditorProject::Active() || s_Job.root != EditorProject::Root())
	{
		EndChecks(true, xr_string());
		return;
	}
	xr_string err;
	const u32 check = s_Job.check;
	const u32 began = ::GetTickCount();
	const bool ok = RunCheck(s_Job, err);
	// a check on the main thread is a frame nobody got: one that takes this long is a bug report
	const u32 spent = ::GetTickCount() - began;
	if (spent > kCheckSlowMs)
		Msg("* [XMS] build: '%s' held the editor for %u ms", kCheckStage[check], spent);
	if (!ok)
	{
		EndChecks(false, err);
		return;
	}
	if (ckDone != s_Job.check) return;
	s_JobThread.join();
	s_JobDone = false;
	s_JobOk = false;
	s_JobOut = SBuildOut();
	s_JobPackage = EditorMod::SPackageResult();
	s_JobDraft = SReleaseDraft();
	s_Job.phase = jpFiles;
	s_JobThread = std::thread(&JobThread);
}

void EditorMod::JobTick()
{
	if (jpIdle == s_Job.phase) return;
	DrainJobLog();
	if (jpChecks == s_Job.phase)
	{
		if (s_Job.frames) { --s_Job.frames; return; }
		if (!s_JobWarmed) return;
		// as many checks as fit a slice: most take no time at all, and a tick each would only add frames to wait for
		const u32 began = ::GetTickCount();
		do
		{
			// an MCP caller decided already: nothing the checks run may put a question up
			if (s_Job.rq.from_mcp)
			{
				XUI::McpScope mcp;
				StepJobChecks();
			}
			else
				StepJobChecks();
		} while (jpChecks == s_Job.phase && ::GetTickCount() - began < kCheckSliceMs);
		return;
	}
	if (!s_JobDone) return;
	s_JobThread.join();

	SJobResult result;
	result.ok			= s_JobOk;
	result.files		= s_JobOut.files;
	result.path			= s_JobOut.path;
	result.err			= s_JobOut.err;
	result.left_out		= s_JobOut.left_out;
	result.stale_names	= s_JobOut.stale_names;
	result.stale_count	= s_JobOut.stale_count;
	result.package		= s_JobPackage;
	xr_string warnings	= s_Job.warnings;
	if (!s_JobOut.warnings.empty()) AddWarning(warnings, s_JobOut.warnings);
	if (result.ok && s_Job.plan.package)
	{
		if (s_Feed.Cancelled())
			{ result.ok = false; result.err = "cancelled"; }
		else if (!FinishRelease(s_Job.plan, s_JobDraft, result.package, result.err, warnings))
			result.ok = false;
		if (!result.ok) result.package = SPackageResult();
	}
	result.warnings		= warnings;
	result.cancelled	= !result.ok && result.err == "cancelled";
	EndJob(result);
}

static void JobRow(SProgressTaskInfoVec& out)
{
	if (jpIdle == s_Job.phase) return;
	const EditorMod::SJobRequest& rq = s_Job.rq;
	SProgressTaskInfo task;
	task.key		= rq.key;
	task.text		= rq.title;
	task.subtitle	= rq.subtitle;
	task.steps		= rq.steps;
	task.stop_label	= rq.stop_label;
	task.stop_tip	= rq.stop_tip;
	task.from_mcp	= rq.from_mcp;
	task.elapsed_ms	= ::GetTickCount() - s_Job.started;
	task.cancelable	= true;
	task.cancel		= [] { EditorMod::StopJob(); };
	task.cancel_requested = s_Feed.Cancelled();
	{
		std::lock_guard<std::mutex> guard(s_Feed.lock);
		task.detail	= s_Feed.stage;
		task.file	= s_Feed.file;
		task.step	= _min(s_Feed.step, int(rq.steps.size()) - 1);
		if (s_Feed.total)
		{
			task.determinate	= true;
			task.current		= float(s_Feed.done);
			task.total			= float(s_Feed.total);
			task.fraction		= clampr(float(double(s_Feed.done) / double(s_Feed.total)), 0.f, 1.f);
			task.numbers		= BytesText(s_Feed.done) + " of " + BytesText(s_Feed.total);
			if (s_Feed.files_total)
				task.numbers += NqUtil::Format(" \xC2\xB7 %u of %u files", _min(s_Feed.files, s_Feed.files_total), s_Feed.files_total);
			const u32 spent = ::GetTickCount() - s_Feed.counted_from;
			if (spent > 2000 && s_Feed.done)
				task.eta_ms = u64(double(spent) * double(s_Feed.total - _min(s_Feed.done, s_Feed.total)) / double(s_Feed.done));
		}
	}
	out.push_back(std::move(task));
}

bool EditorMod::JobRunning()
{
	return jpIdle != s_Job.phase;
}

void EditorMod::StopJob()
{
	if (JobRunning()) s_Feed.cancel = true;
}

const EditorMod::SJobResult& EditorMod::LastJobResult()
{
	return s_JobLast;
}

void EditorMod::JobStatusText(char* dst, u32 size)
{
	dst[0] = 0;
	if (!JobRunning()) return;
	std::lock_guard<std::mutex> guard(s_Feed.lock);
	if (s_Feed.total)
		sprintf_s(dst, size, "%s, %u%%", s_Feed.stage.c_str(), (u32)(double(s_Feed.done) * 100.0 / double(s_Feed.total)));
	else
		strcpy_s(dst, size, s_Feed.stage.c_str());
}

void EditorMod::JobShutdown()
{
	StopJob();
	if (s_JobThread.joinable()) s_JobThread.join();
	if (s_Job.quests) NqExport::DropProjectCheck(s_Job.quests);
	s_Job = SJob();
}

bool EditorMod::StartJob(const SJobRequest& request, xr_string& err)
{
	err.clear();
	if (JobRunning())				{ err = "a build is running: wait for it to finish, or stop it"; return false; }
	if (!EditorProject::Active())	{ err = "no active project"; return false; }
	UIProgressCenter::RegisterSource("module_job", &JobRow);
	s_Job = SJob();
	s_Job.rq		= request;
	s_Job.root		= EditorProject::Root();
	s_Job.started	= ::GetTickCount();
	s_Job.frames	= kFramesFirst;
	s_Job.phase		= jpChecks;
	s_Feed.cancel	= false;
	s_Feed.Stage(kStepCheck, "Reading the project");
	if (s_JobThread.joinable()) s_JobThread.join();
	s_JobWarmed		= false;
	s_JobThread		= std::thread(&WarmThread, s_Job.root);
	return true;
}

//------------------------------------------------------------------------------
// what asks for a build, and what it is told
//------------------------------------------------------------------------------
static void ProjectIni(const xr_string& root, char* dst, u32 size)
{
	sprintf_s(dst, size, "%s\\project.ltx", root.c_str());
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
static void ReportBuild(bool into_game, bool flat, const EditorMod::SJobResult& r, const xr_string& id)
{
	xr_vector<std::pair<xr_string, xr_string>> rows;
	XUI::Strings warnings;
	if (r.ok) rows.emplace_back("Folder", r.path);
	AddBakeNote(r.bake_note, rows, warnings);
	SplitLines(r.warnings, warnings);
	const char* result_id = flat ? "export_result" : "build_result";
	if (!r.ok)
	{
		XUI::Result(result_id, flat ? "Export Failed" : "Build Failed", XUI::Tone::Error, r.err.c_str(), rows, XUI::Strings(), warnings);
		return;
	}
	xr_string headline;
	if (into_game)
	{
		headline = "The module is in the game: " + CountText(r.files, "file", "files") + ". Added objects reach new games and loaded "
			"saves; changes to existing objects need a new game.";
		rows.emplace_back("Console", "xms_enable " + id + "  /  xms_disable " + id);
	}
	else if (flat)
		headline = CountText(r.files, "file", "files") + " exported as a gamedata folder.";
	else
		headline = "The module is built: " + CountText(r.files, "file", "files") + ".";
	XUI::Result(result_id, flat ? "Export Complete" : "Build Complete", warnings.empty() ? XUI::Tone::Success : XUI::Tone::Warning,
		headline.c_str(), rows, r.left_out, warnings, "Open Folder", [path = r.path] { OpenFolder(path); });
}

static bool IsCleanRefusal(const xr_string& err)
{
	return 0 == strncmp(err.c_str(), "target already holds a module", 29);
}

static EditorMod::SJobRequest BuildRequest(EditorMod::EJobKind kind, const xr_string& target, bool clean)
{
	const bool flat = EditorMod::jkExportFlat == kind;
	EditorMod::SManifest m;
	EditorMod::Load(EditorProject::Root(), m);
	EditorMod::SJobRequest rq;
	rq.kind			= kind;
	rq.target		= target;
	rq.clean		= clean;
	rq.key			= "build";
	rq.title		= flat ? "Exporting Gamedata" : "Building Module";
	rq.subtitle		= xr_string(m.name.empty() ? m.id : m.name) + " \xC2\xB7 " + target;
	rq.stop_label	= flat ? "Stop Export" : "Stop Build";
	rq.stop_tip		= flat ? "Stops the export. The folder keeps the files written so far." :
		"Stops the build. The module in the target stays as it was.";
	for (LPCSTR step : { "Check", "Copy" })
		rq.steps.push_back(step);
	if (!flat) rq.steps.push_back("Install");
	return rq;
}

static void StartBuild(EditorMod::EJobKind kind, const xr_string& target, bool clean);

// The build is clean and refused to replace a folder holding files it did not write: the second phase names them and,
// on Yes, runs the build again with `clean`.
static void AskClean(EditorMod::EJobKind kind, const xr_string& target, const EditorMod::SJobResult& r)
{
	const xr_string headline = r.path + " holds " + CountText(r.stale_count, "file", "files") + " this build does not write.";
	XUI::ConfirmSpec spec;
	spec.id = "clean_target";
	spec.title = "Replace Module Files";
	spec.headline = headline.c_str();
	spec.items = r.stale_names;
	spec.total = (u32)r.stale_count;
	spec.consequence = "Deleted for good - this cannot be undone. The folder then holds exactly this build.";
	spec.action = "Delete and Build";
	spec.destructive = true;
	XUI::Confirm(spec, [kind, target](XUI::Answer answer)
	{
		if (answer != XUI::Answer::Yes)
		{
			XUI::Toast(XUI::Tone::Info, "Build cancelled: nothing was deleted.");
			return;
		}
		StartBuild(kind, target, true);
	});
}

static void StartBuild(EditorMod::EJobKind kind, const xr_string& target, bool clean)
{
	const bool flat = EditorMod::jkExportFlat == kind;
	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m) || !EditorMod::ValidateId(m.id.c_str()))
	{
		ReportMissingId(flat ? "Export Flat Gamedata" : "Build Failed");
		return;
	}
	EditorMod::SJobRequest rq = BuildRequest(kind, target, clean);
	rq.done = [kind, flat, target, root = xr_string(EditorProject::Root()), id = m.id](const EditorMod::SJobResult& r)
	{
		if (r.cancelled)
		{
			XUI::Toast(XUI::Tone::Info, flat ? "Export stopped: the folder holds the files written so far." :
				"Build stopped: the module stays as it was.");
			return;
		}
		if (!r.ok && IsCleanRefusal(r.err))
		{
			AskClean(kind, target, r);
			return;
		}
		if (r.ok)
		{
			char ini[MAX_PATH];
			ProjectIni(root, ini, sizeof(ini));
			::WritePrivateProfileStringA("xms", "export_target", target.c_str(), ini);
		}
		ReportBuild(EditorMod::jkBuildIntoGame == kind, flat, r, id);
	};
	xr_string err;
	if (!EditorMod::StartJob(rq, err))
		XUI::Notice(XUI::Tone::Warning, rq.title.c_str(), (err + ".").c_str());
}

void EditorMod::RequestBuildIntoGame()
{
	if (JobRunning())
	{
		UIProgressCenter::ShowCard();
		return;
	}
	if (CanBuildIntoGame())
	{
		StartBuild(jkBuildIntoGame, EditorProject::GameRoot(), false);
		return;
	}
	XUI::Result("build_result", "Build Failed", XUI::Tone::Error,
		"No game is linked. The build writes the module into <game>\\modules\\<id>, where the game reads it: link a game first.",
		xr_vector<std::pair<xr_string, xr_string>>(), XUI::Strings(), XUI::Strings());
}

static void PrepareExport(bool flat)
{
	if (!EditorProject::Active()) return;
	if (EditorMod::JobRunning())
	{
		UIProgressCenter::ShowCard();
		return;
	}
	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m) || !EditorMod::ValidateId(m.id.c_str()))
	{
		ReportMissingId(flat ? "Export Flat Gamedata" : "Build to Folder");
		return;
	}
	s_ExportId = m.id;
	char ini[MAX_PATH];
	ProjectIni(EditorProject::Root(), ini, sizeof(ini));
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
		ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(ModToken::TextSecondary));
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
	ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(ModToken::TextSecondary));
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
		const bool running = EditorMod::JobRunning();
		if (XUI::DialogButton(flat ? "Export" : "Build", XUI::Kind::Primary, true, s_Target[0] && !running,
			running ? "A build is running" : "Pick a folder first"))
		{
			StartBuild(flat ? EditorMod::jkExportFlat : EditorMod::jkBuildToFolder, s_Target, false);
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
static EditorMod::SManifest	s_PkgManifest;		// what the dialog names: id, version, update source
static string_path			s_PkgOutDir			= {};
static char					s_PkgRequires[32]	= {};

void EditorMod::RequestPackageRelease()
{
	if (!EditorProject::Active()) return;
	if (JobRunning())
	{
		UIProgressCenter::ShowCard();
		return;
	}
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

static void ReportPackage(const EditorMod::SManifest& m, const EditorMod::SJobResult& r)
{
	if (r.cancelled)
	{
		XUI::Toast(XUI::Tone::Info, "Packaging stopped: the release folder holds nothing new.");
		return;
	}
	xr_vector<std::pair<xr_string, xr_string>> rows;
	XUI::Strings warnings;
	if (!r.ok)
	{
		AddBakeNote(r.bake_note, rows, warnings);
		XUI::Result("package_result", "Packaging Failed", XUI::Tone::Error, r.err.c_str(), rows, XUI::Strings(), warnings);
		return;
	}
	const EditorMod::SPackageResult& res = r.package;
	rows.emplace_back("Folder", res.out_dir);
	rows.emplace_back("Descriptor", m.id + ".update.ltx");
	rows.emplace_back("File index", res.index.name);
	for (const EditorMod::SPackageFile& p : res.packages)
		rows.emplace_back("Package", NqUtil::Format("%s  (%.1f MB)", p.name.c_str(), double(p.size) / 1048576.0));
	rows.emplace_back("Signed", res.signed_ ? "With the author key " + res.key_id : xr_string("No - the Mod Browser lists signed releases only"));
	AddBakeNote(r.bake_note, rows, warnings);
	if (m.update_github.empty())
		warnings.push_back("No GitHub repository in Mod Settings: the game does not look for updates of this module.");
	SplitLines(r.warnings, warnings);
	// the index addresses the zip by byte offset: a repacked copy is another file
	const xr_string headline = NqUtil::Format("Release %s %s: %s packaged. Upload the files exactly as written to a GitHub release%s "
		"and mark it as the latest - Publish Release does both in one step.", m.id.c_str(), m.version.c_str(),
		CountText(res.files, "file", "files").c_str(), m.update_github.empty() ? "" : (" of " + m.update_github).c_str());
	XUI::Result("package_result", "Release Packaged", warnings.empty() ? XUI::Tone::Success : XUI::Tone::Warning, headline.c_str(),
		rows, res.left_out, warnings, "Open Folder", [dir = res.out_dir] { OpenFolder(dir); });
}

static EditorMod::SJobRequest PackageRequest(const EditorMod::SManifest& m, const xr_string& out_dir, const xr_string& requires_game)
{
	EditorMod::SJobRequest rq;
	rq.kind				= EditorMod::jkPackage;
	rq.out_dir			= out_dir;
	rq.requires_game	= requires_game;
	rq.key				= "package";
	rq.title			= "Packaging Release";
	rq.subtitle			= xr_string(m.name.empty() ? m.id : m.name) + " " + m.version;
	rq.stop_label		= "Stop Packaging";
	rq.stop_tip			= "Stops the packaging. Nothing is written to the release folder before the packages are done.";
	for (LPCSTR step : { "Check", "Copy", "Pack" })
		rq.steps.push_back(step);
	return rq;
}

static void QueuePackage()
{
	s_WantPackage = false;
	EditorMod::SJobRequest rq = PackageRequest(s_PkgManifest, s_PkgOutDir, s_PkgRequires);
	rq.done = [m = s_PkgManifest](const EditorMod::SJobResult& r) { ReportPackage(m, r); };
	xr_string err;
	if (!EditorMod::StartJob(rq, err))
		XUI::Notice(XUI::Tone::Warning, rq.title.c_str(), (err + ".").c_str());
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
		if (XUI::PropertySection("More##package", !s_PkgOutDir[0] || !requires_ok))
		{
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
			XUI::EndPropertySection();
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
		const bool running = EditorMod::JobRunning();
		const char* why = running ? "A build is running" : !s_PkgOutDir[0] ? "Pick the output folder first" : "Fix the game version first";
		if (XUI::DialogButton("Package", XUI::Kind::Primary, true, s_PkgOutDir[0] && requires_ok && !running, why))
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
	if (s_SettingsOpen)
		ShowSettingsTab(false);
	DrawExportDialog(false);
	DrawExportDialog(true);
	DrawPackageDialog();
}

//------------------------------------------------------------------------------
// MCP commands
//------------------------------------------------------------------------------
// [{"name","function","help","args","cheat"}, ...]; a [command.<name>] with no [commands] line has "declared":false
static void JsonCommands(const xr_vector<EditorMod::SCommand>& commands, xr_string& out)
{
	out += "[";
	for (u32 i = 0; i < commands.size(); ++i)
	{
		const EditorMod::SCommand& c = commands[i];
		out += i ? ",{\"name\":" : "{\"name\":";
		NqUtil::JsonString(out, c.name);
		out += ",\"function\":";
		NqUtil::JsonString(out, c.function);
		out += ",\"help\":";
		NqUtil::JsonString(out, c.help);
		out += ",\"args\":";
		NqUtil::JsonString(out, c.args);
		out += ",\"cheat\":";
		NqUtil::JsonBool(out, c.cheat);
		if (!c.declared) out += ",\"declared\":false";
		out += "}";
	}
	out += "]";
}

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
	out += m.disable_game_updates ? "\",\"disable_game_updates\":true,\"website\":\"" : "\",\"disable_game_updates\":false,\"website\":\"";
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
	out += ",\"vars\":[";
	for (u32 i = 0; i < m.vars.size(); ++i)
	{
		out += i ? ",{\"name\":\"" : "{\"name\":\"";
		JsonAppend(out, m.vars[i].name.c_str());
		out += "\",\"value\":\"";
		JsonAppend(out, m.vars[i].value.c_str());
		out += "\"}";
	}
	out += "],\"commands\":";
	JsonCommands(m.commands, out);
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
	if (GrizzlyMCP::GetArg(raw, "id", v, v_size))
	{
		if (!ValidateId(v))
			{ out = "{\"ok\":false,\"error\":\"invalid id (allowed: a-z 0-9 _ . -)\"}"; return; }
		m.id = v;
	}
	if (GrizzlyMCP::GetArg(raw, "name", v, v_size))		m.name = v;
	if (GrizzlyMCP::GetArg(raw, "version", v, v_size))
	{
		if (!ValidateVersion(v))
			{ out = "{\"ok\":false,\"error\":\"invalid version (1-4 numbers separated by dots, e.g. 1.2.0)\"}"; return; }
		m.version = v;
	}
	if (GrizzlyMCP::GetArg(raw, "author", v, v_size))		m.author = v;
	if (GrizzlyMCP::GetArg(raw, "description", v, v_size))
	{
		m.description = v;
		Trim(m.description);
		// the same room the manifest window gives it
		if (m.description.size() >= sizeof(SSettingsForm::description))
			{ out = "{\"ok\":false,\"error\":\"description is too long (4095 bytes at most)\"}"; return; }
	}
	if (NqUtil::ArgBool(raw, "disable_game_updates", false) == NqUtil::ArgBool(raw, "disable_game_updates", true))
		m.disable_game_updates = NqUtil::ArgBool(raw, "disable_game_updates", false);
	if (GrizzlyMCP::GetArg(raw, "website", v, v_size))
	{
		m.website = v;
		Trim(m.website);
		if (!ValidateWebsite(m.website.c_str()))
			{ out = "{\"ok\":false,\"error\":\"invalid website (only https://ap-pro.ru or https://moddb.com links are allowed)\"}"; return; }
	}
	if (GrizzlyMCP::GetArg(raw, "github", v, v_size))
	{
		NormalizeGithubRepo(v, m.update_github);
		if (!ValidateGithubRepo(m.update_github.c_str()))
			{ out = "{\"ok\":false,\"error\":\"invalid github (owner/repo, e.g. Madmer/agroprom-story)\"}"; return; }
	}
	if (GrizzlyMCP::GetArg(raw, "mode", v, v_size))
	{
		if (!ValidateModeId(v))
			{ out = "{\"ok\":false,\"error\":\"invalid mode id (allowed: a-z 0-9 _ . - or empty)\"}"; return; }
		m.mode = v;
	}
	// provides_mode edits touch only the first declared mode; empty id drops it
	if (GrizzlyMCP::GetArg(raw, "provides_mode_id", v, v_size))
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
	if (GrizzlyMCP::GetArg(raw, "provides_mode_title", v, v_size) && !m.provides_modes.empty())
		m.provides_modes[0].title = v;
	// The targeting choice itself. "default" cannot be inferred - a module with
	// no mode set either targets the ordinary game on purpose or was never
	// decided, and only the caller knows which - so it is stated explicitly;
	// picking a mode states it implicitly.
	if (GrizzlyMCP::GetArg(raw, "target", v, v_size))
	{
		if (0 != _stricmp(v, "default") && 0 != _stricmp(v, "existing") && 0 != _stricmp(v, "new"))
			{ out = "{\"ok\":false,\"error\":\"invalid target (allowed: default, existing, new)\"}"; return; }
		m.target = v;
	}
	else if (!m.provides_modes.empty())	m.target = "new";
	else if (!m.mode.empty())			m.target = "existing";
	if (GrizzlyMCP::GetArg(raw, "requires", v, v_size))	SplitList(m.requires_list, v, ',');
	if (GrizzlyMCP::GetArg(raw, "after", v, v_size))		SplitList(m.after, v, ',');
	if (GrizzlyMCP::GetArg(raw, "before", v, v_size))		SplitList(m.before, v, ',');
	if (GrizzlyMCP::GetArg(raw, "conflicts", v, v_size))	SplitList(m.conflicts, v, ',');
	// the whole list at once: one entry per line, or comma separated when on one line
	if (GrizzlyMCP::GetArg(raw, "exclude", v, v_size))
		SplitList(m.exclude, v, strchr(v, '\n') ? '\n' : ',');
	// the global variables, the whole list at once: `name = default` per line (or comma
	// separated on one line); an empty value clears them all
	if (GrizzlyMCP::GetArg(raw, "vars", v, v_size))
	{
		xr_vector<xr_string> rows;
		SplitList(rows, v, strchr(v, '\n') ? '\n' : ',');
		xr_vector<SGlobalVar> vars;
		for (u32 i = 0; i < rows.size(); ++i)
		{
			SGlobalVar gv;
			const size_t eq = rows[i].find('=');
			gv.name = eq == xr_string::npos ? rows[i] : rows[i].substr(0, eq);
			if (eq != xr_string::npos) gv.value = rows[i].substr(eq + 1);
			Trim(gv.name); Trim(gv.value);
			if (gv.name.empty()) continue;
			if (!ValidGlobalName(gv.name.c_str()))
			{
				out = "{\"ok\":false,\"error\":\"a global variable is named by letters, digits and _: '";
				JsonAppend(out, gv.name.c_str());
				out += "'\"}";
				return;
			}
			bool twice = false;
			for (u32 k = 0; k < vars.size(); ++k) if (vars[k].name == gv.name) twice = true;
			if (twice)
			{
				out = "{\"ok\":false,\"error\":\"the global variable '";
				JsonAppend(out, gv.name.c_str());
				out += "' is declared twice\"}";
				return;
			}
			vars.push_back(gv);
		}
		m.vars = vars;
	}
	// the Mod Browser's picture (converted at once) and tags
	if (GrizzlyMCP::GetArg(raw, "tags", v, v_size))
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
	if (GrizzlyMCP::GetArg(raw, "thumbnail", v, v_size))
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

// the answer of a build that was held until it ended (GrizzlyMCP, FollowsJob)
static void AnswerExport(xr_string& out)
{
	const EditorMod::SJobResult& r = EditorMod::LastJobResult();
	if (!r.ok)
	{
		out = "{\"ok\":false,\"error\":\"";
		JsonAppend(out, r.err.c_str());
		out += "\"}";
		return;
	}
	char head[64];
	sprintf_s(head, "{\"ok\":true,\"files\":%d,\"path\":\"", r.files);
	out = head;
	JsonAppendPath(out, r.path.c_str());
	out += "\",\"baked\":\"";
	JsonAppend(out, r.bake_note.c_str());
	// an agent gets the same warnings the dialog shows: a build that shipped a quest
	// naming something that does not exist is not a clean build
	out += "\",\"warnings\":\"";
	JsonAppend(out, r.warnings.c_str());
	// what the build passed by - tooling, [exclude], links - project-relative
	out += "\",\"left_out\":";
	JsonAppendArray(out, r.left_out);
	out += "}";
}

void EditorMod::McpExport(LPCSTR raw, xr_string& out)
{
	if (ArgBool(raw, "answer_only", false))
	{
		AnswerExport(out);
		return;
	}
	if (!EditorProject::Active())
		{ out = "{\"ok\":false,\"error\":\"no active project\"}"; return; }
	char target[MAX_PATH];
	// no target = deploy to the linked game: <game>\modules\<id> is where the
	// game actually reads modules from, so it is the default, not an option
	if (!GrizzlyMCP::GetArg(raw, "target", target, sizeof(target)) || !target[0])
	{
		if (!EditorProject::GameRoot()[0])
			{ out = "{\"ok\":false,\"error\":\"no 'target' and no linked game (see game_link)\"}"; return; }
		strncpy_s(target, sizeof(target), EditorProject::GameRoot(), _TRUNCATE);
	}
	// MCP is the programmatic path: the caller already decided, and it must
	// never stall on a question
	SJobRequest rq = BuildRequest(ArgBool(raw, "flat", false) ? jkExportFlat : jkBuildToFolder, target, true);
	rq.from_mcp = true;
	xr_string err;
	if (StartJob(rq, err))
	{
		out = "{\"ok\":true,\"job_pending\":true}";
		return;
	}
	out = "{\"ok\":false,\"error\":\"";
	JsonAppend(out, err.c_str());
	out += "\"}";
}

void EditorMod::McpExclude(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active())
		{ out = "{\"ok\":false,\"error\":\"no active project\"}"; return; }
	char path[MAX_PATH] = {};
	if (!GrizzlyMCP::GetArg(raw, "path", path, sizeof(path)) || !path[0])
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

void EditorMod::McpCommands(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active())
		{ out = "{\"ok\":false,\"error\":\"no active project\"}"; return; }
	SManifest m;
	if (!Load(EditorProject::Root(), m))
	{
		EnsureScaffold(EditorProject::Root(), EditorProject::Name());
		Load(EditorProject::Root(), m);
	}
	xr_string name;
	NqUtil::ArgString(raw, "name", name);
	name = NqUtil::Trim(name);
	if (!name.empty())
	{
		auto same = [&name](const SCommand& c) { return 0 == _stricmp(c.name.c_str(), name.c_str()); };
		if (NqUtil::ArgBool(raw, "remove", false))
		{
			// every entry of that name: a second line and a section without one go with it
			const size_t before = m.commands.size();
			m.commands.erase(std::remove_if(m.commands.begin(), m.commands.end(), same), m.commands.end());
			if (m.commands.size() == before)
				{ NqUtil::JsonError(out, "there is no command '" + name + "'"); return; }
		}
		else
		{
			if (!ValidCommandName(name.c_str()))
				{ NqUtil::JsonError(out, "a command name is a-z, 0-9 and _ only, 64 at most: '" + name + "'"); return; }
			const auto it = std::find_if(m.commands.begin(), m.commands.end(), same);
			SCommand c = it != m.commands.end() ? *it : SCommand();
			c.name = name;
			xr_string v;
			if (NqUtil::ArgString(raw, "function", v))	c.function = NqUtil::Trim(v);
			if (NqUtil::ArgString(raw, "help", v))		c.help = NqUtil::Trim(v);
			if (NqUtil::ArgString(raw, "args", v))		c.args = NqUtil::Trim(v);
			if (NqUtil::ArgString(raw, "cheat", v))		c.cheat = NqUtil::ArgBool(raw, "cheat", false);
			if (!ValidCommandFunction(c.function.c_str()))
			{
				NqUtil::JsonError(out, "function: <script>.<function> of the project's gamedata\\scripts, e.g. my_mod.status - got '" +
					c.function + "'");
				return;
			}
			c.declared = true;
			if (it != m.commands.end())	*it = c;
			else						m.commands.push_back(c);
		}
		if (!Save(EditorProject::Root(), m))
			{ out = "{\"ok\":false,\"error\":\"can't write mod.ltx\"}"; return; }
	}
	xr_vector<xr_string> functions, problems;
	ScriptFunctions(EditorProject::Root(), functions);
	ValidateCommands(EditorProject::Root(), m, problems);
	out = "{\"ok\":true,\"commands\":";
	JsonCommands(m.commands, out);
	out += ",\"functions\":";
	NqUtil::JsonStringArray(out, functions);
	out += ",\"problems\":[";
	for (u32 i = 0; i < problems.size(); ++i)
	{
		// "E404 commands: '...'" -> {code, text}
		const xr_string& p = problems[i];
		out += i ? ",{\"code\":" : "{\"code\":";
		NqUtil::JsonString(out, p.substr(0, 4));
		out += ",\"text\":";
		NqUtil::JsonString(out, p.size() > 5 ? p.substr(5) : xr_string());
		out += "}";
	}
	out += "]}";
}

static void AnswerPackage(xr_string& out)
{
	const EditorMod::SJobResult& r = EditorMod::LastJobResult();
	if (!r.ok)
	{
		out = "{\"ok\":false,\"error\":\"";
		JsonAppend(out, r.err.c_str());
		out += "\"}";
		return;
	}
	const EditorMod::SPackageResult& res = r.package;
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
	JsonAppend(out, r.bake_note.c_str());
	out += "\",\"warnings\":\"";
	JsonAppend(out, r.warnings.c_str());
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

void EditorMod::McpPackage(LPCSTR raw, xr_string& out)
{
	if (ArgBool(raw, "answer_only", false))
	{
		AnswerPackage(out);
		return;
	}
	if (!EditorProject::Active())
		{ out = "{\"ok\":false,\"error\":\"no active project\"}"; return; }
	// request-sized, so an over-long value reaches the checks instead of reading as absent
	xr_vector<char> arg(xr_strlen(raw) + 1);
	xr_string out_dir, game_version;
	if (GrizzlyMCP::GetArg(raw, "out_dir", arg.data(), (u32)arg.size()))
		out_dir = arg.data();
	// absent = the linked game's version, present but empty = no requirement
	if (GrizzlyMCP::GetArg(raw, "requires_game", arg.data(), (u32)arg.size()))
		game_version = arg.data();
	else
		LinkedGameVersion(game_version);

	// the author key's password, when the key file has one; never echoed or logged
	if (GrizzlyMCP::GetArg(raw, "password", arg.data(), (u32)arg.size()) && arg[0])
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
	SManifest manifest;
	Load(EditorProject::Root(), manifest);
	SJobRequest rq = PackageRequest(manifest, out_dir, game_version);
	rq.from_mcp = true;
	xr_string err;
	if (StartJob(rq, err))
	{
		out = "{\"ok\":true,\"job_pending\":true}";
		return;
	}
	out = "{\"ok\":false,\"error\":\"";
	JsonAppend(out, err.c_str());
	out += "\"}";
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
		if (index == pgCommands) RefreshCommandData();
	}
	// the question, up to be looked at: it opens on the tab's next frame, outside this request
	if (NqUtil::ArgBool(raw, "ask_game_updates", false))
	{
		RequestEditManifest();
		s_SettingsPage = pgGeneral;
		s_AskGameUpdates = true;
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
