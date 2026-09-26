#include "stdafx.h"
#include "EditorPda.h"
#include "EditorPdaInternal.h"
#include "EditorProject.h"
#include "EditorGameContent.h"
#include "EditorModManifest.h"
#include "EditorFileOps.h"
#include "XFinedMCP.h"
#include "Nq/NqUtil.h"
#include "Nq/NqLua.h"

extern "C" {
#include <lua/lua.h>
#include <lua/lauxlib.h>
#include <lua/lualib.h>
#include <lua/luajit.h>
}

//==============================================================================
// the reader: the game's own xms_pda_layout.script
//==============================================================================
namespace
{
	const LPCSTR	kReaderRel		= "scripts\\xms_pda_layout.script";
	const int		kHookPeriod		= 10000;
	const int		kHookLimit		= 800;		// 8M instructions: a big layout needs a few hundred thousand

	xr_string		s_Reader;
	xr_string		s_ReaderSource;
	xr_string		s_ReaderKey;

	void CountHook(lua_State* L, lua_Debug*)
	{
		lua_pushstring(L, "execution limit exceeded");
		lua_error(L);
	}

	xr_string CurrentKey()
	{
		xr_string k = EditorProject::Active() ? EditorProject::Root() : "";
		k += "|";
		if (EditorGameContent::Mounted()) k += EditorGameContent::MountedRoot();
		return k;
	}

	// the linked game's copy wins: the game reads THAT file; the bundled copy is the
	// fallback for an editor with no game, kept byte-identical to the game's
	bool LoadReader(xr_string& err)
	{
		const xr_string key = CurrentKey();
		if (!s_Reader.empty() && key == s_ReaderKey) return true;
		s_Reader.clear();
		s_ReaderSource.clear();
		s_ReaderKey = key;

		xr_string mount_err;
		if (EditorProject::Active() && EditorProject::GameLinked() && EditorGameContent::EnsureMounted(mount_err))
		{
			const int idx = EditorGameContent::Find(kReaderRel);
			u32 size = 0;
			u8* bytes = idx >= 0 ? EditorGameContent::ReadBytes(idx, size) : 0;
			if (bytes)
			{
				s_Reader.assign((const char*)bytes, (const char*)bytes + size);
				EditorGameContent::FreeBytes(bytes);
				s_ReaderSource = xr_string("game:") + kReaderRel;
				return true;
			}
		}
		string_path path;
		sprintf_s(path, "%s\\gamedata\\configs\\pda\\xms_pda_layout.script", NqUtil::SdkRoot());
		xr_string read_err;
		if (!NqLua::ReadFile(path, s_Reader, read_err) || s_Reader.empty())
		{
			err = xr_string("the PDA layout reader was found neither in the linked game nor at ") + path;
			s_Reader.clear();
			return false;
		}
		s_ReaderSource = "bundled:configs\\pda\\xms_pda_layout.script";
		return true;
	}

	// ---- Lua -> C++ ----------------------------------------------------------
	xr_string GetStr(lua_State* L, int t, LPCSTR k)
	{
		lua_getfield(L, t, k);
		xr_string s = lua_type(L, -1) == LUA_TSTRING ? lua_tostring(L, -1) : "";
		lua_pop(L, 1);
		return s;
	}

	bool HasStr(lua_State* L, int t, LPCSTR k)
	{
		lua_getfield(L, t, k);
		const bool r = lua_type(L, -1) == LUA_TSTRING;
		lua_pop(L, 1);
		return r;
	}

	double GetNum(lua_State* L, int t, LPCSTR k, double def)
	{
		lua_getfield(L, t, k);
		const double d = lua_type(L, -1) == LUA_TNUMBER ? lua_tonumber(L, -1) : def;
		lua_pop(L, 1);
		return d;
	}

	bool GetBool(lua_State* L, int t, LPCSTR k, bool def)
	{
		lua_getfield(L, t, k);
		const bool b = lua_type(L, -1) == LUA_TBOOLEAN ? !!lua_toboolean(L, -1) : def;
		lua_pop(L, 1);
		return b;
	}

	bool GetColor(lua_State* L, int t, LPCSTR k, u32& out)
	{
		lua_getfield(L, t, k);
		const bool has = lua_type(L, -1) == LUA_TNUMBER;
		if (has)
		{
			const double d = lua_tonumber(L, -1);
			out = d <= 0.0 ? 0u : d >= 4294967295.0 ? 0xFFFFFFFFu : (u32)d;
		}
		lua_pop(L, 1);
		return has;
	}

	void GetList(lua_State* L, int t, LPCSTR k, xr_vector<xr_string>& out)
	{
		out.clear();
		lua_getfield(L, t, k);
		if (lua_istable(L, -1))
		{
			for (int i = 1;; ++i)
			{
				lua_rawgeti(L, -1, i);
				if (lua_type(L, -1) != LUA_TSTRING) { lua_pop(L, 1); break; }
				out.push_back(lua_tostring(L, -1));
				lua_pop(L, 1);
			}
		}
		lua_pop(L, 1);
	}

	void ReadItem(lua_State* L, int t, EditorPda::SItem& it)
	{
		it.kind = GetStr(L, t, "kind");
		it.layer = GetStr(L, t, "layer");
		it.page = GetStr(L, t, "page");
		it.parent = (int)GetNum(L, t, "parent", 0);
		it.id = GetStr(L, t, "id");
		it.x = (float)GetNum(L, t, "x", 0);
		it.y = (float)GetNum(L, t, "y", 0);
		it.w = (float)GetNum(L, t, "w", 0);
		it.h = (float)GetNum(L, t, "h", 0);
		it.visible = GetBool(L, t, "visible", true);
		GetColor(L, t, "color", it.color);
		it.has_hover = GetColor(L, t, "hover", it.hover);
		it.has_press = GetColor(L, t, "press", it.press);
		it.has_selected = GetColor(L, t, "selected", it.selected);
		it.has_border = GetColor(L, t, "border", it.border);
		GetColor(L, t, "tint", it.tint);
		it.radius = (float)GetNum(L, t, "radius", 0);
		it.border_w = (float)GetNum(L, t, "border_w", 2);
		it.texture = GetStr(L, t, "texture");
		it.stretch = GetBool(L, t, "stretch", true);
		it.has_text = HasStr(L, t, "text");
		it.text = GetStr(L, t, "text");
		it.font = GetStr(L, t, "font");
		it.hint = GetStr(L, t, "hint");
		GetColor(L, t, "text_color", it.text_color);
		it.has_text_hover = GetColor(L, t, "text_hover", it.text_hover);
		it.has_text_selected = GetColor(L, t, "text_selected", it.text_selected);
		const xr_string a = GetStr(L, t, "align"), va = GetStr(L, t, "valign");
		it.align = a.empty() ? 'l' : a[0];
		it.valign = va.empty() ? 'c' : va[0];
		it.wrap = GetBool(L, t, "wrap", false);
		it.icon = GetStr(L, t, "icon");
		it.icon_size = (float)GetNum(L, t, "icon_size", 24);
		GetColor(L, t, "icon_color", it.icon_color);
		it.has_icon_selected = GetColor(L, t, "icon_selected", it.icon_selected);
		GetList(L, t, "action", it.action);
		GetList(L, t, "selected_on", it.selected_on);
		it.row_h = (float)GetNum(L, t, "row_h", 0);
		it.gap = (float)GetNum(L, t, "gap", 0);
		it.empty = GetStr(L, t, "empty");
		it.row = (int)GetNum(L, t, "row", 0);
		it.zoom = GetStr(L, t, "zoom");
	}

	// ---- the preview block, read off the parsed value (the reader ignores it) --
	void ReadPreview(const SNqValue& v, EditorPda::SLayout& out)
	{
		const SNqValue* p = v.Get("preview");
		if (!p || !p->IsTable()) return;
		out.preview_level = p->GetString("level");
		if (const SNqValue* a = p->Get("actor"))
		{
			if (a->IsTable())
			{
				const SNqValue* x = a->Get("x");
				const SNqValue* z = a->Get("z");
				if (!x && a->ArrSize() >= 2) { x = &a->arr[0]; z = &a->arr[a->ArrSize() >= 3 ? 2 : 1]; }
				if (x && z && x->IsNumber() && z->IsNumber())
				{
					out.has_actor = true;
					out.actor_x = (float)x->n;
					out.actor_z = (float)z->n;
				}
			}
		}
		if (const SNqValue* vars = p->Get("vars"))
			for (u32 i = 0; i < vars->keys.size(); ++i)
			{
				const SNqValue& val = vars->vals[i];
				if (val.IsString())			out.preview_vars.push_back(std::make_pair(vars->keys[i], val.s));
				else if (val.IsNumber())	out.preview_vars.push_back(std::make_pair(vars->keys[i], NqUtil::Format("%g", val.n)));
			}
		if (const SNqValue* ms = p->Get("markers"))
			for (u32 i = 0; i < ms->ArrSize(); ++i)
			{
				const SNqValue& m = ms->arr[i];
				if (!m.IsTable()) continue;
				EditorPda::SMarker mk;
				mk.x = (float)m.GetNumber("x", 0);
				mk.z = (float)m.GetNumber("z", 0);
				mk.icon = m.GetString("icon", "ui_xms_icon_pin");
				mk.hint = m.GetString("hint");
				out.markers.push_back(mk);
			}
	}
}

EditorPda::SItem::SItem()
	: parent(0), x(0), y(0), w(0), h(0), visible(true), color(0), hover(0), press(0), selected(0), border(0),
	  tint(0xFFFFFFFF), has_hover(false), has_press(false), has_selected(false), has_border(false), radius(0),
	  border_w(2), stretch(true), has_text(false), text_color(0xFFFFFFFF), text_hover(0), text_selected(0),
	  has_text_hover(false), has_text_selected(false), align('l'), valign('c'), wrap(false), icon_size(24),
	  icon_color(0xFFFFFFFF), icon_selected(0), has_icon_selected(false), row_h(0), gap(0), row(0)
{
}

EditorPda::SLayout::SLayout()
	: ok(false), errors(0), warnings(0), canvas_w(960), canvas_h(680), noise(true), close(true),
	  has_actor(false), actor_x(0), actor_z(0)
{
}

void EditorPdaInternal::DropReader()
{
	s_Reader.clear();
	s_ReaderKey.clear();
}

const EditorPda::SPage* EditorPda::SLayout::FindPage(LPCSTR id) const
{
	for (u32 i = 0; i < pages.size(); ++i)
		if (pages[i].id == id) return &pages[i];
	return 0;
}

LPCSTR EditorPda::ReaderSource()
{
	xr_string err;
	LoadReader(err);
	return s_ReaderSource.c_str();
}

bool EditorPda::Read(LPCSTR text, u32 len, LPCSTR chunk, LPCSTR lang, SLayout& out, xr_string& err, int& line)
{
	out = SLayout();
	line = 0;
	// the loader the game uses for every declarative asset: a syntax error gets its
	// line, code outside strings is refused before anything runs
	SNqValue value;
	NqLua::SError perr;
	if (!NqLua::ParseValue(text, len, chunk, value, perr))
	{
		err = perr.message;
		line = perr.line;
		return false;
	}
	if (!LoadReader(err)) return false;

	lua_State* L = luaL_newstate();
	if (!L) { err = "cannot create a Lua state"; return false; }
	luaL_openlibs(L);
	luaJIT_setmode(L, 0, LUAJIT_MODE_ENGINE | LUAJIT_MODE_OFF);
	bool ok = false;

	// the reader's namespace: what plain Lua gives, and nothing that reaches the machine
	lua_newtable(L);
	const int env = lua_gettop(L);
	static const LPCSTR kGlobals[] = { "string", "table", "math", "pairs", "ipairs", "type", "tostring",
		"tonumber", "pcall", "select", "next", "error", "unpack", "setmetatable", "getmetatable", "rawget",
		"rawset", "assert" };
	for (LPCSTR g : kGlobals)
	{
		lua_getglobal(L, g);
		lua_setfield(L, env, g);
	}

	if (0 != luaL_loadbuffer(L, s_Reader.c_str(), s_Reader.size(), "=xms_pda_layout.script"))
		err = xr_string("the PDA layout reader does not compile: ") + lua_tostring(L, -1);
	else
	{
		lua_pushvalue(L, env);
		lua_setfenv(L, -2);
		if (0 != lua_pcall(L, 0, 0, 0))
			err = xr_string("the PDA layout reader failed: ") + lua_tostring(L, -1);
		else
		{
			lua_getfield(L, env, "normalize");
			const xr_string cname = xr_string("=") + (chunk && chunk[0] ? chunk : "pdaasset");
			if (!lua_isfunction(L, -1))
				err = "the PDA layout reader has no normalize()";
			else if (0 != luaL_loadbuffer(L, text, len, cname.c_str()))
				err = lua_tostring(L, -1);
			else
			{
				// the asset evaluates in an empty environment, exactly as in the game
				lua_newtable(L);
				lua_setfenv(L, -2);
				lua_sethook(L, CountHook, LUA_MASKCOUNT, kHookPeriod * kHookLimit);
				if (0 != lua_pcall(L, 0, 1, 0))
					err = xr_string("code outside strings: ") + lua_tostring(L, -1);
				else
				{
					lua_newtable(L);
					lua_pushstring(L, lang && lang[0] ? lang : "rus");
					lua_setfield(L, -2, "lang");
					if (0 != lua_pcall(L, 2, 1, 0))
						err = xr_string("the PDA layout reader failed: ") + lua_tostring(L, -1);
					else if (!lua_istable(L, -1))
						err = "the PDA layout reader returned nothing";
					else
					{
						const int r = lua_gettop(L);
						out.ok = GetBool(L, r, "ok", false);
						out.errors = (int)GetNum(L, r, "errors", 0);
						out.warnings = (int)GetNum(L, r, "warnings", 0);
						out.id = GetStr(L, r, "id");
						out.title = GetStr(L, r, "title");
						out.start = GetStr(L, r, "start");
						lua_getfield(L, r, "canvas");
						if (lua_istable(L, -1))
						{
							out.canvas_w = (float)GetNum(L, lua_gettop(L), "w", 960);
							out.canvas_h = (float)GetNum(L, lua_gettop(L), "h", 680);
						}
						lua_pop(L, 1);
						lua_getfield(L, r, "device");
						if (lua_istable(L, -1))
						{
							out.noise = GetBool(L, lua_gettop(L), "noise", true);
							out.close = GetBool(L, lua_gettop(L), "close", true);
						}
						lua_pop(L, 1);
						lua_getfield(L, r, "keys");
						if (lua_istable(L, -1))
						{
							const int k = lua_gettop(L);
							out.key_map = GetStr(L, k, "map");
							out.key_tasks = GetStr(L, k, "tasks");
							out.key_contacts = GetStr(L, k, "contacts");
						}
						lua_pop(L, 1);
						lua_getfield(L, r, "vars");
						if (lua_istable(L, -1))
						{
							lua_pushnil(L);
							while (lua_next(L, -2))
							{
								if (lua_type(L, -2) == LUA_TSTRING && lua_type(L, -1) == LUA_TSTRING)
									out.vars.push_back(std::make_pair(xr_string(lua_tostring(L, -2)), xr_string(lua_tostring(L, -1))));
								lua_pop(L, 1);
							}
							std::sort(out.vars.begin(), out.vars.end());
						}
						lua_pop(L, 1);
						lua_getfield(L, r, "pages");
						for (int i = 1; lua_istable(L, -1); ++i)
						{
							lua_rawgeti(L, -1, i);
							if (!lua_istable(L, -1)) { lua_pop(L, 1); break; }
							const int p = lua_gettop(L);
							SPage page;
							page.id = GetStr(L, p, "id");
							page.title = GetStr(L, p, "title");
							page.stock = GetStr(L, p, "stock");
							out.pages.push_back(page);
							lua_pop(L, 1);
						}
						lua_pop(L, 1);
						lua_getfield(L, r, "items");
						for (int i = 1; lua_istable(L, -1); ++i)
						{
							lua_rawgeti(L, -1, i);
							if (!lua_istable(L, -1)) { lua_pop(L, 1); break; }
							SItem it;
							ReadItem(L, lua_gettop(L), it);
							out.items.push_back(it);
							lua_pop(L, 1);
						}
						lua_pop(L, 1);
						lua_getfield(L, r, "problems");
						for (int i = 1; lua_istable(L, -1); ++i)
						{
							lua_rawgeti(L, -1, i);
							if (!lua_istable(L, -1)) { lua_pop(L, 1); break; }
							const int p = lua_gettop(L);
							SProblem pr;
							pr.code = GetStr(L, p, "code");
							pr.path = GetStr(L, p, "path");
							pr.message = GetStr(L, p, "message");
							out.problems.push_back(pr);
							lua_pop(L, 1);
						}
						lua_pop(L, 1);
						ok = true;
					}
				}
			}
		}
	}
	lua_close(L);
	if (ok) ReadPreview(value, out);
	return ok;
}

bool EditorPda::ReadFile(LPCSTR path, LPCSTR lang, SLayout& out, xr_string& err, int& line)
{
	xr_string text;
	line = 0;
	if (!NqLua::ReadFile(path, text, err)) return false;
	return Read(text.c_str(), (u32)text.size(), NqUtil::BaseName(path).c_str(), lang, out, err, line);
}

xr_string EditorPda::Template(LPCSTR id, LPCSTR title)
{
	xr_string safe = id && id[0] ? id : "app";
	for (size_t i = 0; i < safe.size(); ++i)
	{
		char& c = safe[i];
		if (c >= 'A' && c <= 'Z') c = char(c - 'A' + 'a');
		else if (!((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_')) c = '_';
	}
	// the title goes into a Lua string literal
	xr_string name;
	for (LPCSTR p = title && title[0] ? title : safe.c_str(); *p; ++p)
	{
		if (*p == '"' || *p == '\\') name += '\\';
		if (*p != '\n' && *p != '\r') name += *p;
	}
	// longer than NqUtil::Format's buffer: assembled, not formatted
	xr_string t = R"lua(-- The PDA interface of this module: what the PDA shows instead of its stock tabs.
-- Pick it in Mod > PDA Interface (or MCP pda_use). Format: docs/pda/PDA_FORMAT.md.
return {
  pda = 1,
  id = "@ID@",
  title = "@TITLE@",
  start = "home",
  keys = { map = "map" },

  colors = { bg = "#101114", bar = "#18191E", card = "#1F2128", line = "#2E323C",
             accent = "#FFD21E", text = "#F4F4F6", muted = "#9AA0AB" },

  styles = {
    tab = { kind = "button", color = "none", hover = "#FFFFFF0D", text_color = "muted",
            text_selected = "accent", icon_color = "muted", icon_selected = "accent",
            font = "letterica16", align = "center", valign = "bottom", icon_size = 26 },
  },

  chrome = {
    { kind = "rect", x = 0, y = 0, w = 960, h = 680, color = "bg" },
    { kind = "rect", x = 0, y = 0, w = 960, h = 64, color = "bar" },
    { kind = "text", x = 24, y = 0, w = 600, h = 64, text = "{title}", font = "letterica25", color = "text" },
    { kind = "text", x = 800, y = 0, w = 136, h = 64, text = "{time}", color = "muted", align = "right" },
    { kind = "rect", x = 0, y = 604, w = 960, h = 76, color = "bar" },
    { style = "tab", x = 0, y = 604, w = 320, h = 76, text = "Home", icon = "ui_xms_icon_home", page = "home" },
    { style = "tab", x = 320, y = 604, w = 320, h = 76, text = "Map", icon = "ui_xms_icon_map", page = "map" },
    { style = "tab", x = 640, y = 604, w = 320, h = 76, text = "Journal", icon = "ui_xms_icon_list", page = "journal" },
  },

  pages = {
    { id = "home", title = "Home", widgets = {
      { kind = "group", x = 24, y = 88, w = 912, h = 160, radius = 18, color = "card", widgets = {
        { kind = "text", x = 24, y = 20, w = 860, h = 34, text = "Hello, {actor}", font = "letterica25", color = "text" },
        { kind = "text", x = 24, y = 60, w = 860, h = 80, text = "Your screens go here.", color = "muted", valign = "top", wrap = true },
      } },
    } },
    { id = "map", title = "Map", widgets = {
      { kind = "map", x = 16, y = 80, w = 928, h = 508, color = "#0B0C0F" },
    } },
    { id = "journal", title = "Journal", stock = "eptTasks" },
  },
}
)lua";
	const size_t i = t.find("@ID@");
	t.replace(i, 4, safe);
	const size_t k = t.find("@TITLE@");
	t.replace(k, 7, name);
	return t;
}

//==============================================================================
// the project
//==============================================================================
bool EditorPda::IsPdaPath(LPCSTR path)
{
	return NqUtil::Extension(path) == ".pdaasset";
}

void EditorPda::List(LPCSTR project_root, xr_vector<xr_string>& files)
{
	files.clear();
	if (!project_root || !project_root[0]) return;
	// what the build ships is what counts: tooling and editor scratch are never walked
	NqUtil::ListFiles(project_root, ".pdaasset", files, &EditorMod::OutsideBuild);
	std::sort(files.begin(), files.end());
}

bool EditorPda::SamePath(LPCSTR a, LPCSTR b)
{
	if (!a || !b) return false;
	for (;; ++a, ++b)
	{
		const char ca = *a == '/' ? '\\' : (char)tolower((u8)*a);
		const char cb = *b == '/' ? '\\' : (char)tolower((u8)*b);
		if (ca != cb) return false;
		if (!ca) return true;
	}
}

xr_string EditorPda::UsedLayout()
{
	if (!EditorProject::Active()) return xr_string();
	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m)) return xr_string();
	return m.pda_layout;
}

bool EditorPda::Use(LPCSTR rel, xr_string& err)
{
	if (!EditorProject::Active()) { err = "no active project"; return false; }
	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m)) { err = "mod.ltx not found in the project"; return false; }
	xr_string value;
	if (rel && rel[0])
	{
		string_path abs;
		if (!EditorFileOps::ResolveInProject(rel, abs, err)) return false;
		if (!IsPdaPath(abs)) { err = "a PDA layout is a .pdaasset file"; return false; }
		if (!NqUtil::FileExists(abs)) { err = NqUtil::Format("%s does not exist", rel); return false; }
		value = NqUtil::ProjectRelative(abs);
		if (EditorMod::OutsideBuild(EditorProject::Root(), value.c_str()))
			{ err = NqUtil::Format("%s is left out of the build ([exclude] or tooling) - the game would never see it", value.c_str()); return false; }
	}
	// one spelling in mod.ltx, whatever separators the caller used
	for (size_t i = 0; i < value.size(); ++i)
		if (value[i] == '/') value[i] = '\\';
	m.pda_layout = value;
	if (!EditorMod::Save(EditorProject::Root(), m)) { err = "mod.ltx cannot be written"; return false; }
	return true;
}

void EditorPda::MissingTextures(const SLayout& l, xr_vector<xr_string>& out)
{
	out.clear();
	xr_vector<xr_string> seen;
	for (u32 i = 0; i < l.items.size(); ++i)
	{
		const SItem& it = l.items[i];
		for (int k = 0; k < 2; ++k)
		{
			const xr_string& t = k ? it.icon : it.texture;
			if (t.empty() || std::find(seen.begin(), seen.end(), t) != seen.end()) continue;
			seen.push_back(t);
			if (!EditorPdaInternal::TextureKnown(t.c_str())) out.push_back(t);
		}
	}
}

void EditorPda::ValidateForBuild(LPCSTR project_root, xr_vector<xr_string>& errors, xr_vector<xr_string>& warnings)
{
	EditorMod::SManifest m;
	if (!EditorMod::Load(project_root, m) || m.pda_layout.empty()) return;
	const xr_string rel = m.pda_layout;
	string_path abs;
	sprintf_s(abs, "%s\\%s", project_root, rel.c_str());
	if (!NqUtil::FileExists(abs))
		{ errors.push_back(NqUtil::Format("[pda] layout = %s: the file does not exist", rel.c_str())); return; }
	if (EditorMod::OutsideBuild(project_root, rel.c_str()))
		{ errors.push_back(NqUtil::Format("[pda] layout = %s: the build leaves the file out", rel.c_str())); return; }
	SLayout l;
	xr_string err;
	int line = 0;
	if (!ReadFile(abs, "rus", l, err, line))
	{
		errors.push_back(line ? NqUtil::Format("%s:%d: %s", rel.c_str(), line, err.c_str())
							  : NqUtil::Format("%s: %s", rel.c_str(), err.c_str()));
		return;
	}
	for (u32 i = 0; i < l.problems.size(); ++i)
	{
		const SProblem& p = l.problems[i];
		const xr_string line_text = NqUtil::Format("%s %s %s: %s", rel.c_str(), p.code.c_str(),
			p.path.empty() ? "layout" : p.path.c_str(), p.message.c_str());
		(p.code.size() && p.code[0] == 'E' ? errors : warnings).push_back(line_text);
	}
	xr_vector<xr_string> missing;
	if (EditorGameContent::Mounted()) MissingTextures(l, missing);
	for (u32 i = 0; i < missing.size(); ++i)
		warnings.push_back(NqUtil::Format("%s W320: texture '%s' is neither in the linked game nor in the project",
			rel.c_str(), missing[i].c_str()));
}

bool EditorPda::CreateIn(LPCSTR folder, string_path& created, xr_string& err)
{
	for (int n = 1; n < 1000; ++n)
	{
		if (1 == n)	sprintf_s(created, "%s\\NewPda.pdaasset", folder);
		else		sprintf_s(created, "%s\\NewPda%d.pdaasset", folder, n);
		if (NqUtil::FileExists(created)) continue;
		const xr_string id = n == 1 ? xr_string("new_pda") : NqUtil::Format("new_pda%d", n);
		if (!NqLua::WriteFile(created, Template(id.c_str(), "New PDA"), err)) return false;
		EditorPdaInternal::FileWritten(created);
		return true;
	}
	err = "no free NewPda name in the folder";
	return false;
}

void EditorPda::ShowModuleLayout()
{
	if (!EditorProject::Active()) return;
	xr_string rel = UsedLayout();
	if (rel.empty())
	{
		xr_vector<xr_string> files;
		List(EditorProject::Root(), files);
		if (!files.empty()) rel = NqUtil::ProjectRelative(files[0].c_str());
	}
	if (rel.empty())
	{
		// nothing yet: the template, made the module's layout
		EditorMod::SManifest m;
		EditorMod::Load(EditorProject::Root(), m);
		const xr_string id = m.id.empty() ? xr_string("app") : m.id;
		xr_string safe;
		for (size_t i = 0; i < id.size(); ++i) safe += (isalnum((u8)id[i]) ? (char)tolower((u8)id[i]) : '_');
		string_path abs, folder;
		sprintf_s(folder, "%s\\pda", EditorProject::Root());
		CreateDirectoryA(folder, 0);
		sprintf_s(abs, "%s\\%s.pdaasset", folder, safe.c_str());
		xr_string err;
		if (!NqLua::WriteFile(abs, Template(safe.c_str(), m.name.empty() ? safe.c_str() : m.name.c_str()), err)) return;
		rel = NqUtil::ProjectRelative(abs);
		Use(rel.c_str(), err);
	}
	string_path abs;
	sprintf_s(abs, "%s\\%s", EditorProject::Root(), rel.c_str());
	Open(abs);
}

//==============================================================================
// MCP
//==============================================================================
namespace
{
	void JsonProblems(xr_string& out, const EditorPda::SLayout& l, const xr_vector<xr_string>& missing)
	{
		out += "[";
		bool first = true;
		for (u32 i = 0; i < l.problems.size(); ++i)
		{
			if (!first) out += ",";
			first = false;
			out += "{\"code\":";
			NqUtil::JsonString(out, l.problems[i].code);
			out += ",\"path\":";
			NqUtil::JsonString(out, l.problems[i].path);
			out += ",\"message\":";
			NqUtil::JsonString(out, l.problems[i].message);
			out += "}";
		}
		for (u32 i = 0; i < missing.size(); ++i)
		{
			if (!first) out += ",";
			first = false;
			out += "{\"code\":\"W320\",\"path\":\"\",\"message\":";
			NqUtil::JsonString(out, NqUtil::Format("texture '%s' is neither in the linked game nor in the project", missing[i].c_str()));
			out += "}";
		}
		out += "]";
	}

	void JsonOutline(xr_string& out, const EditorPda::SLayout& l)
	{
		out += "{\"id\":";
		NqUtil::JsonString(out, l.id);
		out += ",\"title\":";
		NqUtil::JsonString(out, l.title);
		out += ",\"start\":";
		NqUtil::JsonString(out, l.start);
		out += NqUtil::Format(",\"canvas\":{\"w\":%g,\"h\":%g},\"device\":{\"noise\":%s,\"close\":%s}", l.canvas_w,
			l.canvas_h, l.noise ? "true" : "false", l.close ? "true" : "false");
		out += ",\"keys\":{\"map\":";
		NqUtil::JsonString(out, l.key_map);
		out += ",\"tasks\":";
		NqUtil::JsonString(out, l.key_tasks);
		out += ",\"contacts\":";
		NqUtil::JsonString(out, l.key_contacts);
		out += "},\"pages\":[";
		for (u32 i = 0; i < l.pages.size(); ++i)
		{
			const EditorPda::SPage& p = l.pages[i];
			int widgets = 0;
			for (u32 k = 0; k < l.items.size(); ++k)
				if (l.items[k].page == p.id && l.items[k].parent == 0) ++widgets;
			if (i) out += ",";
			out += "{\"id\":";
			NqUtil::JsonString(out, p.id);
			out += ",\"title\":";
			NqUtil::JsonString(out, p.title);
			if (!p.stock.empty())
			{
				out += ",\"stock\":";
				NqUtil::JsonString(out, p.stock);
			}
			out += NqUtil::Format(",\"widgets\":%d}", widgets);
		}
		out += "],\"vars\":{";
		for (u32 i = 0; i < l.vars.size(); ++i)
		{
			if (i) out += ",";
			NqUtil::JsonString(out, l.vars[i].first);
			out += ":";
			NqUtil::JsonString(out, l.vars[i].second);
		}
		// the addressable items: what show:/hide:/toggle: and xms.pda name
		out += "},\"ids\":[";
		bool first = true;
		for (u32 i = 0; i < l.items.size(); ++i)
		{
			const EditorPda::SItem& it = l.items[i];
			if (it.id.empty()) continue;
			if (!first) out += ",";
			first = false;
			out += "{\"id\":";
			NqUtil::JsonString(out, it.id);
			out += ",\"kind\":";
			NqUtil::JsonString(out, it.kind);
			out += ",\"layer\":";
			NqUtil::JsonString(out, it.layer);
			if (!it.page.empty())
			{
				out += ",\"page\":";
				NqUtil::JsonString(out, it.page);
			}
			out += NqUtil::Format(",\"visible\":%s}", it.visible ? "true" : "false");
		}
		out += NqUtil::Format("],\"items\":%u}", (u32)l.items.size());
	}

	// one separator from here on: the caller may write pda/app.pdaasset
	void Backslashes(xr_string& s)
	{
		for (size_t i = 0; i < s.size(); ++i)
			if (s[i] == '/') s[i] = '\\';
	}

	bool ResolvePath(LPCSTR raw, xr_string& abs, xr_string& rel, xr_string& err)
	{
		xr_string path;
		if (!NqUtil::ArgString(raw, "path", path) || path.empty())
		{
			// the layout the module uses is the default subject
			path = EditorPda::UsedLayout();
			if (path.empty()) { err = "missing 'path' (and the module uses no PDA layout yet)"; return false; }
		}
		string_path buf;
		if (!EditorFileOps::ResolveInProject(path.c_str(), buf, err)) return false;
		abs = buf;
		Backslashes(abs);
		if (!EditorPda::IsPdaPath(abs.c_str())) { err = "a PDA layout is a .pdaasset file"; return false; }
		rel = NqUtil::ProjectRelative(abs.c_str());
		return true;
	}

	void CreateDirChain(LPCSTR folder)
	{
		string_path p;
		xr_strcpy(p, sizeof(p), folder);
		for (char* s = p; *s; ++s)
		{
			if (*s != '\\' || s == p || s[-1] == ':') continue;
			*s = 0;
			CreateDirectoryA(p, 0);
			*s = '\\';
		}
		CreateDirectoryA(p, 0);
	}
}

void EditorPda::McpList(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active()) { NqUtil::JsonError(out, "no active project"); return; }
	xr_vector<xr_string> files;
	List(EditorProject::Root(), files);
	const xr_string used = UsedLayout();
	xr_string lang = "rus";
	NqUtil::ArgString(raw, "lang", lang);
	out = "{\"ok\":true,\"used\":";
	NqUtil::JsonPath(out, used.c_str());
	out += ",\"reader\":";
	NqUtil::JsonPath(out, ReaderSource());
	out += ",\"layouts\":[";
	for (u32 i = 0; i < files.size(); ++i)
	{
		SLayout l;
		xr_string err;
		int line = 0;
		const bool ok = ReadFile(files[i].c_str(), lang.c_str(), l, err, line);
		const xr_string rel = NqUtil::ProjectRelative(files[i].c_str());
		if (i) out += ",";
		out += "{\"path\":";
		NqUtil::JsonPath(out, rel.c_str());
		out += NqUtil::Format(",\"used\":%s", SamePath(rel.c_str(), used.c_str()) ? "true" : "false");
		if (!ok)
		{
			out += ",\"error\":";
			NqUtil::JsonString(out, line ? NqUtil::Format("line %d: %s", line, err.c_str()) : err);
		}
		else
		{
			out += ",\"id\":";
			NqUtil::JsonString(out, l.id);
			out += ",\"title\":";
			NqUtil::JsonString(out, l.title);
			out += NqUtil::Format(",\"pages\":%u,\"items\":%u,\"errors\":%d,\"warnings\":%d", (u32)l.pages.size(),
				(u32)l.items.size(), l.errors, l.warnings);
		}
		out += "}";
	}
	out += "]}";
}

void EditorPda::McpGet(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active()) { NqUtil::JsonError(out, "no active project"); return; }
	xr_string abs, rel, err;
	if (!ResolvePath(raw, abs, rel, err)) { NqUtil::JsonError(out, err); return; }
	xr_string text;
	if (!NqLua::ReadFile(abs.c_str(), text, err)) { NqUtil::JsonError(out, err); return; }
	xr_string lang = "rus";
	NqUtil::ArgString(raw, "lang", lang);
	SLayout l;
	int line = 0;
	const bool ok = Read(text.c_str(), (u32)text.size(), NqUtil::BaseName(abs.c_str()).c_str(), lang.c_str(), l, err, line);
	out = "{\"ok\":true,\"path\":";
	NqUtil::JsonPath(out, rel.c_str());
	out += NqUtil::Format(",\"used\":%s", SamePath(rel.c_str(), UsedLayout().c_str()) ? "true" : "false");
	out += ",\"text\":";
	NqUtil::JsonString(out, text);
	if (!ok)
	{
		out += ",\"parse_error\":";
		NqUtil::JsonString(out, err);
		out += NqUtil::Format(",\"line\":%d}", line);
		return;
	}
	out += ",\"layout\":";
	JsonOutline(out, l);
	xr_vector<xr_string> missing;
	if (EditorGameContent::Mounted()) MissingTextures(l, missing);
	out += NqUtil::Format(",\"errors\":%d,\"warnings\":%d,\"problems\":", l.errors, l.warnings + (int)missing.size());
	JsonProblems(out, l, missing);
	out += ",\"reader\":";
	NqUtil::JsonPath(out, ReaderSource());
	out += "}";
}

void EditorPda::McpWrite(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active()) { NqUtil::JsonError(out, "no active project"); return; }
	xr_string text, path, id, title;
	NqUtil::ArgString(raw, "text", text);
	NqUtil::ArgString(raw, "path", path);
	NqUtil::ArgString(raw, "id", id);
	NqUtil::ArgString(raw, "title", title);
	if (path.empty())
	{
		if (id.empty()) { NqUtil::JsonError(out, "missing 'path' (or 'id' for a new layout under pda\\)"); return; }
		path = "pda\\" + id + ".pdaasset";
	}
	string_path buf;
	xr_string err;
	if (!EditorFileOps::ResolveInProject(path.c_str(), buf, err)) { NqUtil::JsonError(out, err); return; }
	xr_string abs = buf;
	Backslashes(abs);
	// a bare name gets the family's extension, the way the browser's rename does
	if (NqUtil::Extension(abs.c_str()).empty()) abs += ".pdaasset";
	if (!IsPdaPath(abs.c_str())) { NqUtil::JsonError(out, "a PDA layout is a .pdaasset file"); return; }
	const bool existed = NqUtil::FileExists(abs.c_str());
	// no text: a new layout from the template (never over an existing file)
	if (text.empty())
	{
		if (existed) { NqUtil::JsonError(out, "missing 'text' (the whole .pdaasset body)"); return; }
		text = Template(id.empty() ? NqUtil::BaseName(abs.c_str()).c_str() : id.c_str(), title.c_str());
	}
	SLayout l;
	int line = 0;
	if (!Read(text.c_str(), (u32)text.size(), NqUtil::BaseName(abs.c_str()).c_str(), "rus", l, err, line))
	{
		out = "{\"ok\":false,\"error\":";
		NqUtil::JsonString(out, line ? NqUtil::Format("line %d: %s", line, err.c_str()) : err);
		out += NqUtil::Format(",\"line\":%d,\"written\":false}", line);
		return;
	}
	string_path folder;
	xr_strcpy(folder, sizeof(folder), abs.c_str());
	if (char* slash = strrchr(folder, '\\')) *slash = 0;
	CreateDirChain(folder);
	if (!NqLua::WriteFile(abs.c_str(), text, err)) { NqUtil::JsonError(out, err); return; }
	EditorPdaInternal::FileWritten(abs.c_str());
	// the first layout of a module is the one it uses, unless the caller says otherwise
	bool used_now = false;
	if (NqUtil::ArgBool(raw, "use", UsedLayout().empty()))
	{
		xr_string uerr;
		used_now = Use(NqUtil::ProjectRelative(abs.c_str()).c_str(), uerr);
	}
	xr_vector<xr_string> missing;
	if (EditorGameContent::Mounted()) MissingTextures(l, missing);
	out = "{\"ok\":true,\"path\":";
	NqUtil::JsonPath(out, NqUtil::ProjectRelative(abs.c_str()).c_str());
	out += NqUtil::Format(",\"created\":%s,\"used\":%s,\"layout\":", existed ? "false" : "true",
		SamePath(NqUtil::ProjectRelative(abs.c_str()).c_str(), UsedLayout().c_str()) ? "true" : "false");
	JsonOutline(out, l);
	out += NqUtil::Format(",\"errors\":%d,\"warnings\":%d,\"problems\":", l.errors, l.warnings + (int)missing.size());
	JsonProblems(out, l, missing);
	out += NqUtil::Format(",\"use_changed\":%s}", used_now ? "true" : "false");
}

void EditorPda::McpDelete(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active()) { NqUtil::JsonError(out, "no active project"); return; }
	xr_string abs, rel, err;
	if (!ResolvePath(raw, abs, rel, err)) { NqUtil::JsonError(out, err); return; }
	if (!NqUtil::FileExists(abs.c_str())) { NqUtil::JsonError(out, NqUtil::Format("%s does not exist", rel.c_str())); return; }
	EditorFileOps::SReport rep;
	EditorFileOps::Delete(abs.c_str(), false, rep);
	if (NqUtil::FileExists(abs.c_str())) { NqUtil::JsonError(out, NqUtil::Format("%s could not be deleted", rel.c_str())); return; }
	EditorPdaInternal::FileWritten(abs.c_str());
	// the module does not keep naming a file that is gone
	bool cleared = false;
	if (SamePath(rel.c_str(), UsedLayout().c_str()))
	{
		xr_string uerr;
		cleared = Use("", uerr);
	}
	out = "{\"ok\":true,\"deleted\":";
	NqUtil::JsonPath(out, rel.c_str());
	out += NqUtil::Format(",\"use_cleared\":%s}", cleared ? "true" : "false");
}

void EditorPda::McpUse(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active()) { NqUtil::JsonError(out, "no active project"); return; }
	xr_string path, err;
	NqUtil::ArgString(raw, "path", path);
	if (!Use(path.c_str(), err)) { NqUtil::JsonError(out, err); return; }
	out = "{\"ok\":true,\"used\":";
	NqUtil::JsonPath(out, UsedLayout().c_str());
	out += "}";
}

void EditorPda::McpValidate(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active()) { NqUtil::JsonError(out, "no active project"); return; }
	xr_string text, lang = "rus", err;
	NqUtil::ArgString(raw, "lang", lang);
	xr_string label = "text";
	// a text to check before writing it, or a file
	if (!NqUtil::ArgString(raw, "text", text) || text.empty())
	{
		xr_string abs, rel;
		if (!ResolvePath(raw, abs, rel, err)) { NqUtil::JsonError(out, err); return; }
		if (!NqLua::ReadFile(abs.c_str(), text, err)) { NqUtil::JsonError(out, err); return; }
		label = rel;
	}
	SLayout l;
	int line = 0;
	if (!Read(text.c_str(), (u32)text.size(), NqUtil::BaseName(label.c_str()).c_str(), lang.c_str(), l, err, line))
	{
		out = "{\"ok\":true,\"valid\":false,\"parse_error\":";
		NqUtil::JsonString(out, err);
		out += NqUtil::Format(",\"line\":%d}", line);
		return;
	}
	xr_vector<xr_string> missing;
	if (EditorGameContent::Mounted()) MissingTextures(l, missing);
	out = NqUtil::Format("{\"ok\":true,\"valid\":%s,\"errors\":%d,\"warnings\":%d,\"problems\":", l.ok ? "true" : "false",
		l.errors, l.warnings + (int)missing.size());
	JsonProblems(out, l, missing);
	out += ",\"layout\":";
	JsonOutline(out, l);
	out += "}";
}
