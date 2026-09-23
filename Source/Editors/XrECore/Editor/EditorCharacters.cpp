#include "stdafx.h"
#include "EditorCharacters.h"
#include "EditorProject.h"
#include "EditorModManifest.h"
#include "EditorGameContent.h"
#include "ui_main.h"
#include "Nq/NqLua.h"
#include "Nq/NqUtil.h"
#include "Nq/NqPickers.h"
#include "EditorFileOps.h"
#include "EDX11Utils.h"

namespace
{
//------------------------------------------------------------------------------
// small helpers
//------------------------------------------------------------------------------
void CreateDirChain(const char* folder)
{
	if (!folder || !folder[0])	return;
	string_path path;
	xr_strcpy(path, sizeof(path), folder);
	for (char* p = path; *p; ++p)
	{
		if (*p != '\\' && *p != '/')	continue;
		const char saved = *p;
		*p = 0;
		if (path[0] && path[1] == ':' && !path[2]) { *p = saved; continue; }	// "D:"
		::CreateDirectoryA(path, 0);
		*p = saved;
	}
	::CreateDirectoryA(path, 0);
}

bool ValidId(const xr_string& id)
{
	if (id.empty() || id.size() > 48)	return false;
	for (u32 i = 0; i < id.size(); ++i)
	{
		const char c = id[i];
		const bool ok = (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_';
		if (!ok)	return false;
	}
	return !(id[0] >= '0' && id[0] <= '9');
}

// A text is a plain string or a table keyed by language. The generated string
// table needs one file per language, so both shapes are walked the same way.
void TextLanguages(const SNqValue& v, xr_vector<xr_string>& langs)
{
	if (v.IsString())
	{
		langs.push_back("rus");
		return;
	}
	if (!v.IsTable())	return;
	for (u32 i = 0; i < v.keys.size(); ++i)
		if (v.vals[i].IsString())
			langs.push_back(v.keys[i]);
}

xr_string TextIn(const SNqValue& v, LPCSTR lang)
{
	if (v.IsString())					return v.s;
	if (!v.IsTable())					return xr_string();
	if (const SNqValue* x = v.Get(lang))
		if (x->IsString())				return x->s;
	if (const SNqValue* r = v.Get("rus"))
		if (r->IsString())				return r->s;
	for (u32 i = 0; i < v.vals.size(); ++i)
		if (v.vals[i].IsString())		return v.vals[i].s;
	return xr_string();
}

// What is actually STORED for a language - no falling back to another one.
// Generation wants the fallback (an English install reading a Russian-only
// character still gets a name); a form must not, or "not set" and "set to the
// same thing" look identical and the author cannot tell them apart.
xr_string TextRaw(const SNqValue& v, LPCSTR lang)
{
	if (v.IsString())					return 0 == strcmp(lang, "rus") ? v.s : xr_string();
	if (!v.IsTable())					return xr_string();
	if (const SNqValue* x = v.Get(lang))
		if (x->IsString())				return x->s;
	return xr_string();
}

bool TextEmpty(const SNqValue& v)
{
	xr_vector<xr_string> langs;
	TextLanguages(v, langs);
	for (u32 i = 0; i < langs.size(); ++i)
		if (!NqUtil::Trim(TextIn(v, langs[i].c_str())).empty())	return false;
	return true;
}

// a Lua string literal, escaped the way the canonical writer escapes one
xr_string LuaString(const xr_string& s)
{
	return NqLua::WriteValue(SNqValue::String(s), 0);
}

void XmlEscape(const xr_string& in, xr_string& out)
{
	out.clear();
	for (u32 i = 0; i < in.size(); ++i)
	{
		switch (in[i])
		{
		case '&':	out += "&amp;";		break;
		case '<':	out += "&lt;";		break;
		case '>':	out += "&gt;";		break;
		case '"':	out += "&quot;";	break;
		default:	out += in[i];		break;
		}
	}
}

xr_string Xml(const xr_string& utf8)
{
	xr_string escaped;
	XmlEscape(utf8, escaped);
	return NqUtil::Utf8ToCp1251(escaped.c_str());
}

LPCSTR ModuleId(xr_string& storage)
{
	EditorMod::SManifest m;
	if (EditorProject::Root() && EditorMod::Load(EditorProject::Root(), m))
		storage = m.id;
	return storage.c_str();
}
} // namespace

//------------------------------------------------------------------------------
// the model
//------------------------------------------------------------------------------
EditorCharacters::SCharacter::SCharacter()	{ Reset(); }

void EditorCharacters::SCharacter::Reset()
{
	file.clear();	id.clear();	base.clear();
	abstract_		= false;
	name = SNqValue();	bio = SNqValue();
	// Empty means UNSET, not "the default": a default written here would
	// override whatever the base said. The defaults are applied once, after the
	// chain is flattened.
	community.clear();
	visual.clear();	icon.clear();	start_dialog.clear();
	spec_rank.clear();
	rank			= 0;
	reputation		= 0;
	has_rank		= false;
	has_reputation	= false;
	has_money		= false;
	money_min		= 0;
	money_max		= 0;
	money_infinite	= false;
	supplies.clear();
	squad			= true;
	has_squad		= false;
}

//------------------------------------------------------------------------------
// reading
//------------------------------------------------------------------------------
bool EditorCharacters::Parse(LPCSTR text, u32 len, LPCSTR chunk_name, SCharacter& c, xr_string& err)
{
	c.Reset();

	SNqValue		v;
	NqLua::SError	lua_err;
	if (!NqLua::ParseValue(text, len, chunk_name, v, lua_err))
	{
		err = lua_err.line
			? NqUtil::Format("line %d: %s", lua_err.line, lua_err.message.c_str())
			: lua_err.message;
		return false;
	}
	if (!v.IsTable())
	{
		err = "a character file must return one table";
		return false;
	}

	c.id			= v.GetString("id");
	c.base			= v.GetString("base");
	c.abstract_		= v.GetBool("abstract", false);
	if (const SNqValue* n = v.Get("name"))	c.name = *n;
	if (const SNqValue* b = v.Get("bio"))	c.bio  = *b;
	c.community		= v.GetString("community");
	c.visual		= v.GetString("visual");
	c.icon			= v.GetString("icon");
	c.start_dialog	= v.GetString("start_dialog");
	c.spec_rank		= v.GetString("spec_rank");
	if (const SNqValue* sq = v.Get("squad"))
		if (sq->IsBool())	{ c.squad = sq->b; c.has_squad = true; }

	if (const SNqValue* r = v.Get("rank"))
		if (r->IsNumber())	{ c.rank = int(r->n); c.has_rank = true; }
	if (const SNqValue* r = v.Get("reputation"))
		if (r->IsNumber())	{ c.reputation = int(r->n); c.has_reputation = true; }

	if (const SNqValue* m = v.Get("money"))
		if (m->IsTable())
		{
			c.has_money			= true;
			c.money_min			= int(m->GetNumber("min", 0.0));
			c.money_max			= int(m->GetNumber("max", double(c.money_min)));
			c.money_infinite	= m->GetBool("infinite", false);
			if (c.money_max < c.money_min)	c.money_max = c.money_min;
		}

	if (const SNqValue* s = v.Get("supplies"))
		for (u32 i = 0; i < s->arr.size(); ++i)
		{
			const SNqValue& row = s->arr[i];
			SSupply item;
			// a bare string is the one-item shorthand
			if (row.IsString())			item.section = row.s;
			else if (row.IsTable())
			{
				item.section	= row.GetString("section");
				item.count		= int(row.GetNumber("count", 1.0));
			}
			// 0 is meaningful: it takes an inherited row away again
			if (item.count < 0)			item.count = 0;
			if (!item.section.empty())	c.supplies.push_back(item);
		}

	return true;
}

bool EditorCharacters::Load(LPCSTR path, SCharacter& c, xr_string& err)
{
	xr_string text;
	if (!NqLua::ReadFile(path, text, err))	return false;
	if (!Parse(text.c_str(), (u32)text.size(), NqUtil::BaseName(path).c_str(), c, err))	return false;
	c.file = path;
	// the file name is the id when the file does not name one - the same rule
	// quests follow, so a renamed file renames the character
	if (c.id.empty())
	{
		xr_string base = NqUtil::BaseName(path);
		const size_t dot = base.find_last_of('.');
		if (dot != xr_string::npos)	base.erase(dot);
		c.id = NqText::IdFromName(base.c_str());
	}
	return true;
}

//------------------------------------------------------------------------------
// writing — canonical: fixed key order, fixed layout
//------------------------------------------------------------------------------
xr_string EditorCharacters::Write(const SCharacter& c)
{
	xr_string s;
	s += "-- XFined Editor character. Built into the module's own character_desc,\n";
	s += "-- profile and spawn section - see docs/MOD_QUICKSTART.md.\n";
	s += "return {\n";
	s += "\tnpc       = 1,\n";
	s += "\tid        = " + LuaString(c.id) + ",\n";
	if (!c.base.empty())
		s += "\tbase      = " + LuaString(c.base) + ",\n";
	if (c.abstract_)
		s += "\tabstract  = true,\n";
	if (!TextEmpty(c.name))
		s += "\tname      = " + NqLua::WriteValue(c.name, 1) + ",\n";
	if (!TextEmpty(c.bio))
		s += "\tbio       = " + NqLua::WriteValue(c.bio, 1) + ",\n";
	if (!c.community.empty())
		s += "\tcommunity = " + LuaString(c.community) + ",\n";
	if (!c.visual.empty())
		s += "\tvisual    = " + LuaString(c.visual) + ",\n";
	if (!c.icon.empty())
		s += "\ticon      = " + LuaString(c.icon) + ",\n";
	if (!c.spec_rank.empty())
		s += "\tspec_rank = " + LuaString(c.spec_rank) + ",\n";
	if (c.has_rank)
		s += NqUtil::Format("\trank      = %d,\n", c.rank);
	if (c.has_reputation)
		s += NqUtil::Format("\treputation = %d,\n", c.reputation);
	if (c.has_money)
	{
		s += NqUtil::Format("\tmoney     = { min = %d, max = %d", c.money_min, c.money_max);
		if (c.money_infinite)	s += ", infinite = true";
		s += " },\n";
	}
	if (!c.supplies.empty())
	{
		s += "\tsupplies  = {\n";
		for (u32 i = 0; i < c.supplies.size(); ++i)
		{
			s += "\t\t{ section = " + LuaString(c.supplies[i].section);
			if (c.supplies[i].count != 1)
				s += NqUtil::Format(", count = %d", c.supplies[i].count);
			s += " },\n";
		}
		s += "\t},\n";
	}
	if (!c.start_dialog.empty())
		s += "\tstart_dialog = " + LuaString(c.start_dialog) + ",\n";
	if (c.has_squad && !c.squad)
		s += "\tsquad     = false,\n";
	s += "}\n";
	return s;
}

bool EditorCharacters::Save(LPCSTR path, const SCharacter& c, xr_string& err)
{
	EditorCharacters::Invalidate();
	return NqLua::WriteFile(path, Write(c), err);
}

//------------------------------------------------------------------------------
// who the project has (cached)
//------------------------------------------------------------------------------
namespace
{
xr_vector<EditorCharacters::SBrief>	s_Briefs;
xr_string							s_BriefsRoot;
bool								s_BriefsValid = false;
}

void EditorCharacters::Invalidate()	{ s_BriefsValid = false; }

const xr_vector<EditorCharacters::SBrief>& EditorCharacters::Briefs()
{
	const xr_string root = EditorProject::Active() ? xr_string(EditorProject::Root()) : xr_string();
	if (s_BriefsValid && root == s_BriefsRoot)	return s_Briefs;
	s_Briefs.clear();
	s_BriefsRoot	= root;
	s_BriefsValid	= true;
	if (root.empty())	return s_Briefs;

	xr_vector<SCharacter>	all;
	xr_vector<xr_string>	ignored;
	LoadResolved(root.c_str(), all, ignored);
	for (u32 i = 0; i < all.size(); ++i)
	{
		SBrief b;
		b.id		= all[i].id;
		b.name		= TextIn(all[i].name, "rus");
		b.file		= NqUtil::ProjectRelative(all[i].file.c_str());
		b.abstract_	= all[i].abstract_;
		s_Briefs.push_back(b);
	}
	return s_Briefs;
}

xr_string EditorCharacters::Template(LPCSTR id)
{
	SCharacter c;
	c.id		= id && id[0] ? NqText::IdFromName(id) : xr_string("new_character");
	// a placeholder the author replaces - the name is what the player reads,
	// so it belongs to the mod's language, not to the editor's
	c.community	= "stalker";
	c.name		= SNqValue::String("New character");
	c.visual	= "actors\\stalker_neutral\\stalker_neutral_1";
	// a loner's face of Dead Air Refined - the old SoC id is no picture there
	c.icon		= "ui_inGame2_neutral_2";
	c.has_rank	= true;
	c.rank		= 300;
	return Write(c);
}

//------------------------------------------------------------------------------
// the project
//------------------------------------------------------------------------------
xr_string EditorCharacters::GameId(LPCSTR module_id, LPCSTR char_id)
{
	xr_string id;
	if (module_id && module_id[0])	{ id = module_id; id += "_"; }
	id += char_id ? char_id : "";
	return id;
}

void EditorCharacters::List(LPCSTR project_root, xr_vector<xr_string>& files)
{
	files.clear();
	if (!project_root || !project_root[0])	return;
	// the characters of the project are the ones its build generates: a character in
	// an excluded folder, the editor's scratch or tooling is no character of the module
	NqUtil::ListFiles(project_root, ".npcasset", files, &EditorMod::OutsideBuild);
}

void EditorCharacters::LoadAll(LPCSTR project_root, xr_vector<SCharacter>& out,
							   xr_vector<xr_string>& problems)
{
	out.clear();
	xr_vector<xr_string> files;
	List(project_root, files);
	for (u32 i = 0; i < files.size(); ++i)
	{
		SCharacter	c;
		xr_string	err;
		if (!Load(files[i].c_str(), c, err))
		{
			problems.push_back("E100 " + NqUtil::ProjectRelative(files[i].c_str()) + ": " + err);
			continue;
		}
		out.push_back(c);
	}
	// a stable order keeps the generated files diff-friendly
	for (u32 i = 0; i + 1 < out.size(); ++i)
		for (u32 j = i + 1; j < out.size(); ++j)
			if (out[j].id < out[i].id)
			{
				SCharacter t = out[i]; out[i] = out[j]; out[j] = t;
			}

	for (u32 i = 0; i + 1 < out.size(); ++i)
		if (out[i].id == out[i + 1].id)
			problems.push_back("E101 two characters share the id '" + out[i].id + "': " +
				NqUtil::ProjectRelative(out[i].file.c_str()) + " and " +
				NqUtil::ProjectRelative(out[i + 1].file.c_str()));
}

//------------------------------------------------------------------------------
// inheritance
//
// The tenth guard of a mod is the same twenty lines as the first. `base` names
// another character to start from: every field this one does not set comes from
// there, and the kits are MERGED - the base's items plus these, a row with the
// same section replacing the inherited one, and count = 0 taking one away.
//
// A character marked `abstract` exists only to be inherited from: it never
// becomes a profile, a spawn section or a line in the string table, so it needs
// neither a name nor a model.
//------------------------------------------------------------------------------
namespace
{
const EditorCharacters::SCharacter* ById(const xr_vector<EditorCharacters::SCharacter>& all,
										 const xr_string& id)
{
	for (u32 i = 0; i < all.size(); ++i)
		if (all[i].id == id) return &all[i];
	return 0;
}

// `src` wins wherever it actually says something; everything else stays.
void ApplyOver(EditorCharacters::SCharacter& dst, const EditorCharacters::SCharacter& src)
{
	if (!src.name.IsNil())			dst.name		= src.name;
	if (!src.bio.IsNil())			dst.bio			= src.bio;
	if (!src.community.empty())		dst.community	= src.community;
	if (!src.visual.empty())		dst.visual		= src.visual;
	if (!src.icon.empty())			dst.icon		= src.icon;
	if (!src.start_dialog.empty())	dst.start_dialog= src.start_dialog;
	if (!src.spec_rank.empty())		dst.spec_rank	= src.spec_rank;
	if (src.has_rank)				{ dst.rank = src.rank; dst.has_rank = true; }
	if (src.has_reputation)			{ dst.reputation = src.reputation; dst.has_reputation = true; }
	if (src.has_money)
	{
		dst.has_money		= true;
		dst.money_min		= src.money_min;
		dst.money_max		= src.money_max;
		dst.money_infinite	= src.money_infinite;
	}
	if (src.has_squad)				{ dst.squad = src.squad; dst.has_squad = true; }

	// the kit: the inherited rows keep their place, a repeat of a section takes
	// over its count, a new section is appended, and zero means "not this one"
	for (u32 i = 0; i < src.supplies.size(); ++i)
	{
		bool replaced = false;
		for (u32 j = 0; j < dst.supplies.size(); ++j)
			if (0 == _stricmp(dst.supplies[j].section.c_str(), src.supplies[i].section.c_str()))
			{
				dst.supplies[j].count = src.supplies[i].count;
				replaced = true;
				break;
			}
		if (!replaced) dst.supplies.push_back(src.supplies[i]);
	}
}

void DropEmptyRows(xr_vector<EditorCharacters::SSupply>& rows)
{
	for (u32 i = 0; i < rows.size(); )
		if (rows[i].count < 1)	rows.erase(rows.begin() + i);
		else					++i;
}
} // namespace

void EditorCharacters::Resolve(xr_vector<SCharacter>& all, xr_vector<xr_string>& problems)
{
	const xr_vector<SCharacter> raw = all;		// the chain is walked over the originals

	for (u32 i = 0; i < all.size(); ++i)
	{
		SCharacter& c	= all[i];
		const xr_string where = c.file.empty()
			? c.id
			: NqUtil::ProjectRelative(c.file.c_str());

		if (!c.base.empty())
		{
			xr_vector<const SCharacter*>	chain;
			xr_string						next = c.base;
			bool							broken = false;

			while (!next.empty())
			{
				if (next == c.id)
				{
					problems.push_back("E112 " + where + ": '" + c.id + "' inherits from itself");
					broken = true;
					break;
				}
				const SCharacter* b = ById(raw, next);
				if (!b)
				{
					problems.push_back("E111 " + where + ": there is no character '" + next +
						"' to inherit from");
					broken = true;
					break;
				}
				bool seen = false;
				for (u32 k = 0; k < chain.size(); ++k)
					if (chain[k] == b) seen = true;
				if (seen || chain.size() > 32)
				{
					problems.push_back("E112 " + where + ": the base chain of '" + c.id +
						"' comes back to where it started");
					broken = true;
					break;
				}
				chain.push_back(b);
				next = b->base;
			}
			if (broken)
			{
				DropEmptyRows(c.supplies);
				continue;
			}

			SCharacter merged;
			merged.Reset();
			// deepest base first, then each one closer, then the character itself
			for (int k = (int)chain.size() - 1; k >= 0; --k)
				ApplyOver(merged, *chain[k]);
			ApplyOver(merged, c);

			merged.file			= c.file;
			merged.id			= c.id;
			merged.base			= c.base;
			merged.abstract_	= c.abstract_;
			c = merged;
		}

		DropEmptyRows(c.supplies);
		// the defaults, once, where they cannot override a base
		if (c.community.empty())	c.community = "stalker";
		if (c.spec_rank.empty())	c.spec_rank = "novice";
	}
}

void EditorCharacters::LoadResolved(LPCSTR project_root, xr_vector<SCharacter>& out,
									xr_vector<xr_string>& problems)
{
	LoadAll(project_root, out, problems);
	Resolve(out, problems);
}

//------------------------------------------------------------------------------
// validation
//------------------------------------------------------------------------------
void EditorCharacters::Validate(const SCharacter& c, bool linked_game, xr_vector<xr_string>& problems)
{
	const xr_string where = c.file.empty()
		? xr_string("character")
		: NqUtil::ProjectRelative(c.file.c_str());

	if (!ValidId(c.id))
		problems.push_back("E102 " + where + ": id '" + c.id +
			"' is not usable (a-z, 0-9 and _, not starting with a digit, up to 48)");

	// A template is not a character: it exists to be inherited from, so it owes
	// the player nothing.
	if (!c.abstract_ && TextEmpty(c.name))
		problems.push_back("E103 " + where + ": no name - it is what the player reads");

	if (!c.abstract_ && c.visual.empty())
		problems.push_back("E104 " + where + ": no visual - an NPC without a model cannot spawn");
	else if (!c.visual.empty() && linked_game && EditorGameContent::Mounted())
	{
		// Looked up in the linked game's content index, which is where an
		// actor model lives; a model the project brings itself is staged by
		// the build and is not expected here, hence a warning and not an error.
		xr_string rel = "meshes\\" + c.visual;
		if (NqUtil::Extension(rel.c_str()).empty())	rel += ".ogf";
		if (EditorGameContent::Find(rel.c_str()) < 0)
			problems.push_back("W105 " + where + ": the linked game has no model at '" +
				rel + "' - check it is staged in the project");
	}

	// The engine refuses a character of a community it has never heard of. A module
	// can bring a community of its own, which the linked game knows nothing about -
	// hence a warning and not an error, like the model above.
	if (!c.community.empty() && linked_game && !NqPickers::Index(NqPickers::tCommunity).empty() &&
		!NqPickers::Known(NqPickers::tCommunity, c.community.c_str()))
		problems.push_back("W114 " + where + ": the linked game has no community '" + c.community +
			"' - the engine refuses a character of a community it does not know");

	// Picked from lists, but the text and MCP path can name anything: what the linked
	// game does not list is said. Warnings - a module may bring its own portrait,
	// dialog or item, and the linked game knows nothing of those.
	if (linked_game && NqPickers::Available())
	{
		if (!c.icon.empty() && !NqPickers::Known(NqPickers::tIcon, c.icon.c_str()))
			problems.push_back("W115 " + where + ": no portrait '" + c.icon +
				"' in the linked game or the project - the player sees an empty frame");
		if (!c.start_dialog.empty() && !NqPickers::Known(NqPickers::tDialog, c.start_dialog.c_str()))
			problems.push_back("W116 " + where + ": no dialog '" + c.start_dialog +
				"' in the linked game or the project");
		for (u32 i = 0; i < c.supplies.size(); ++i)
			if (!c.supplies[i].section.empty() && !NqPickers::Known(NqPickers::tItem, c.supplies[i].section.c_str()))
				problems.push_back("W117 " + where + ": the linked game has no item '" + c.supplies[i].section +
					"' - right only when the module brings that section, otherwise the game stops when the NPC spawns");
	}

	// A text that cp1251 cannot carry silently loses characters in game: the
	// string table, like every other game config, is cp1251.
	xr_vector<xr_string> langs;
	TextLanguages(c.name, langs);
	TextLanguages(c.bio, langs);
	for (u32 i = 0; i < langs.size(); ++i)
	{
		const xr_string text = TextIn(c.name, langs[i].c_str()) + TextIn(c.bio, langs[i].c_str());
		xr_string bad;
		if (!text.empty() && !NqUtil::Cp1251Covers(text, bad))
			problems.push_back("W107 " + where + ": '" + bad +
				"' does not exist in cp1251 and will not reach the player");
	}

	for (u32 i = 0; i < c.supplies.size(); ++i)
	{
		if (c.supplies[i].section.empty())
			problems.push_back("E108 " + where + ": a supply row without a section");
		// a zero is how an inherited row is taken away, and it is gone by now;
		// a negative one never meant anything
		if (c.supplies[i].count < 1)
			problems.push_back("E109 " + where + ": supply '" + c.supplies[i].section +
				"' asks for less than one item");
	}

	if (c.has_money && c.money_max < c.money_min)
		problems.push_back("E110 " + where + ": money max is below money min");
}

void EditorCharacters::ValidateAll(LPCSTR project_root, bool linked_game, xr_vector<xr_string>& problems)
{
	xr_vector<SCharacter> all;
	LoadResolved(project_root, all, problems);
	for (u32 i = 0; i < all.size(); ++i)
		Validate(all[i], linked_game, problems);
}

//------------------------------------------------------------------------------
// the build
//------------------------------------------------------------------------------
void EditorCharacters::GeneratedFiles(LPCSTR module_id, xr_vector<xr_string>& rel)
{
	if (!module_id || !module_id[0])	return;
	const xr_string id = module_id;
	rel.push_back("gamedata\\configs\\gameplay\\character_desc_" + id + ".xml");
	rel.push_back("gamedata\\configs\\gameplay\\npc_profile_" + id + ".xml");
	rel.push_back("gamedata\\configs\\xms\\" + id + "_npc.ltx");
	rel.push_back("patch\\" + id + "_characters.ltxp");
	// the string tables, one per language the game ships a folder for
	static const char* kLangs[] = { "rus", "eng" };
	for (int i = 0; i < 2; ++i)
		rel.push_back(xr_string("gamedata\\configs\\text\\") + kLangs[i] + "\\" + id + "_npc.xml");
}

namespace
{
void RemoveGenerated(LPCSTR project_root, LPCSTR module_id)
{
	xr_vector<xr_string> rel;
	EditorCharacters::GeneratedFiles(module_id, rel);
	for (u32 i = 0; i < rel.size(); ++i)
	{
		string_path p;
		sprintf_s(p, "%s\\%s", project_root, rel[i].c_str());
		if (::DeleteFileA(p))
			Msg("* [XMS] no characters left - removed %s", p);
	}
}

bool WriteGenerated(LPCSTR project_root, const xr_string& rel, const xr_string& body, xr_string& err)
{
	string_path path;
	sprintf_s(path, "%s\\%s", project_root, rel.c_str());
	string_path folder;
	xr_strcpy(folder, sizeof(folder), path);
	if (char* slash = strrchr(folder, '\\'))	*slash = 0;
	CreateDirChain(folder);

	FILE* f = 0;
	if (fopen_s(&f, path, "wb") || !f)
	{
		err = xr_string("cannot write ") + path;
		return false;
	}
	if (!body.empty())	fwrite(body.c_str(), 1, body.size(), f);
	fclose(f);
	return true;
}
} // namespace

bool EditorCharacters::Generate(LPCSTR project_root, LPCSTR module_id, int& written, xr_string& err)
{
	written = 0;
	if (!project_root || !project_root[0])	return true;
	if (!module_id || !module_id[0])
	{
		err = "the module has no id - Mod > Edit Manifest";
		return false;
	}

	xr_vector<SCharacter>	all;
	xr_vector<xr_string>	problems;
	LoadResolved(project_root, all, problems);
	// A character that does not parse must stop the build: shipping half a
	// roster produces a module whose quests point at NPCs that do not exist.
	for (u32 i = 0; i < problems.size(); ++i)
		if (0 == strncmp(problems[i].c_str(), "E", 1))
		{
			err = problems[i];
			return false;
		}

	// templates are not characters: they exist for the others to inherit from
	for (u32 i = 0; i < all.size(); )
		if (all[i].abstract_)	all.erase(all.begin() + i);
		else					++i;

	if (all.empty())
	{
		RemoveGenerated(project_root, module_id);
		return true;
	}

	for (u32 i = 0; i < all.size(); ++i)
	{
		xr_vector<xr_string> own;
		Validate(all[i], false, own);
		for (u32 j = 0; j < own.size(); ++j)
			if (0 == strncmp(own[j].c_str(), "E", 1))
			{
				err = own[j];
				return false;
			}
	}

	const xr_string mod = module_id;

	// ---- the characters ---------------------------------------------------
	xr_string desc = "<?xml version=\"1.0\" encoding=\"windows-1251\"?>\r\n\r\n<xml>\r\n";
	xr_string prof = "<?xml version=\"1.0\" encoding=\"windows-1251\"?>\r\n\r\n<xml>\r\n";
	xr_string ltx  = "; Spawn and squad sections of this module's own characters.\r\n"
					 "; Generated by XFined Editor - edit the .npcasset files instead.\r\n\r\n";

	for (u32 i = 0; i < all.size(); ++i)
	{
		const SCharacter&	c	= all[i];
		const xr_string		gid	= GameId(mod.c_str(), c.id.c_str());

		desc += "\t<specific_character id=\"" + Xml(gid) + "\" no_random=\"1\">\r\n";
		desc += "\t\t<name>" + Xml(gid + "_name") + "</name>\r\n";
		if (!TextEmpty(c.bio))
			desc += "\t\t<bio>" + Xml(gid + "_bio") + "</bio>\r\n";
		if (!c.icon.empty())
			desc += "\t\t<icon>" + Xml(c.icon) + "</icon>\r\n";
		desc += "\t\t<class>" + Xml(gid) + "</class>\r\n";
		desc += "\t\t<community>" + Xml(c.community) + "</community>\r\n";
		desc += "\t\t<visual>" + Xml(c.visual) + "</visual>\r\n";
		if (c.has_rank)
			desc += NqUtil::Format("\t\t<rank>%d</rank>\r\n", c.rank);
		if (c.has_reputation)
			desc += NqUtil::Format("\t\t<reputation>%d</reputation>\r\n", c.reputation);
		if (c.has_money)
			desc += NqUtil::Format("\t\t<money min=\"%d\" max=\"%d\" infinitive=\"%d\"/>\r\n",
				c.money_min, c.money_max, c.money_infinite ? 1 : 0);
		// The kit. "[spawn] \n" then one line per entry is the engine's own
		// format - a count is written only when it is more than one, exactly
		// as the stock files do.
		desc += "\t\t<supplies>\r\n\t\t\t[spawn] \\n\r\n";
		for (u32 s = 0; s < c.supplies.size(); ++s)
		{
			desc += "\t\t\t" + Xml(c.supplies[s].section);
			if (c.supplies[s].count != 1)
				desc += NqUtil::Format(" = %d", c.supplies[s].count);
			desc += " \\n\r\n";
		}
		desc += "\t\t</supplies>\r\n";
		if (!c.start_dialog.empty())
			desc += "\t\t<start_dialog>" + Xml(c.start_dialog) + "</start_dialog>\r\n";
		desc += "\t</specific_character>\r\n";

		// The profile names the specific character outright: `class` alone
		// would let the engine pick a RANDOM character of that class, and a
		// quest NPC has to be the one the author wrote.
		prof += "\t<character id=\"" + Xml(gid) + "\">\r\n";
		prof += "\t\t<class>" + Xml(gid) + "</class>\r\n";
		prof += "\t\t<specific_character>" + Xml(gid) + "</specific_character>\r\n";
		prof += "\t</character>\r\n";

		// $spawn is what puts the character in the editor's spawn roster and
		// lets a quest create it with spawn.object; the squad section is what
		// spawn.squad needs.
		ltx += "[" + gid + "]:stalker\r\n";
		ltx += "$spawn                = \"npc\\" + mod + "\\" + c.id + "\"\r\n";
		ltx += "character_profile     = " + gid + "\r\n";
		ltx += "community             = " + c.community + "\r\n";
		if (!c.spec_rank.empty())
			ltx += "spec_rank             = " + c.spec_rank + "\r\n";
		ltx += "\r\n";
		if (c.squad)
		{
			ltx += "[" + gid + "_squad]:online_offline_group\r\n";
			ltx += "faction               = " + c.community + "\r\n";
			ltx += "npc                   = " + gid + "\r\n\r\n";
		}
	}
	desc += "</xml>\r\n";
	prof += "</xml>\r\n";

	if (!WriteGenerated(project_root, "gamedata\\configs\\gameplay\\character_desc_" + mod + ".xml", desc, err))	return false;
	if (!WriteGenerated(project_root, "gamedata\\configs\\gameplay\\npc_profile_" + mod + ".xml", prof, err))		return false;
	if (!WriteGenerated(project_root, "gamedata\\configs\\xms\\" + mod + "_npc.ltx", ltx, err))					return false;

	// ---- the names --------------------------------------------------------
	// Every language any character names gets its own file; a language a
	// character does not have falls back to rus inside TextIn, so no string is
	// ever missing from a file that exists.
	xr_vector<xr_string> langs;
	for (u32 i = 0; i < all.size(); ++i)
	{
		TextLanguages(all[i].name, langs);
		TextLanguages(all[i].bio, langs);
	}
	for (u32 i = 0; i < langs.size(); ++i)
		for (u32 j = i + 1; j < langs.size(); )
			if (langs[j] == langs[i])	langs.erase(langs.begin() + j);
			else						++j;
	if (langs.empty())	langs.push_back("rus");

	static const char* kKnownLangs[] = { "rus", "eng" };
	for (int k = 0; k < 2; ++k)
	{
		const xr_string lang = kKnownLangs[k];
		bool wanted = false;
		for (u32 i = 0; i < langs.size(); ++i)
			if (langs[i] == lang)	wanted = true;
		const xr_string rel = "gamedata\\configs\\text\\" + lang + "\\" + mod + "_npc.xml";
		if (!wanted)
		{
			string_path stale;
			sprintf_s(stale, "%s\\%s", project_root, rel.c_str());
			::DeleteFileA(stale);
			continue;
		}

		xr_string st = "<?xml version=\"1.0\" encoding=\"windows-1251\"?>\r\n\r\n<string_table>\r\n";
		for (u32 i = 0; i < all.size(); ++i)
		{
			const xr_string gid = GameId(mod.c_str(), all[i].id.c_str());
			st += "\t<string id=\"" + Xml(gid + "_name") + "\">\r\n\t\t<text>" +
				  Xml(TextIn(all[i].name, lang.c_str())) + "</text>\r\n\t</string>\r\n";
			if (!TextEmpty(all[i].bio))
				st += "\t<string id=\"" + Xml(gid + "_bio") + "\">\r\n\t\t<text>" +
					  Xml(TextIn(all[i].bio, lang.c_str())) + "</text>\r\n\t</string>\r\n";
		}
		st += "</string_table>\r\n";
		if (!WriteGenerated(project_root, rel, st, err))	return false;
	}

	// ---- the three lines that make the game read all of it ----------------
	// '+' appends to a list instead of replacing it (XMS/xms_ltx.cpp), which
	// is the whole reason a module can have characters without overwriting
	// system.ltx and fighting every other module that wants the same.
	xr_string patch;
	patch += "; Makes the game read this module's characters. Generated by XFined Editor.\r\n";
	patch += "; '+' APPENDS to the list - nothing of the game or of another module is lost.\r\n\r\n";
	patch += "[profiles]\r\n";
	patch += "+files = npc_profile_" + mod + "\r\n";
	patch += "+specific_characters_files = character_desc_" + mod + "\r\n";
	if (!WriteGenerated(project_root, "patch\\" + mod + "_characters.ltxp", patch, err))	return false;

	written = (int)all.size();
	Msg("* [XMS] characters: %d generated into the module", written);
	return true;
}

//------------------------------------------------------------------------------
// MCP — every UI capability has a programmatic twin (CLAUDE.md), and a
// character is a text file, so the AI path is the primary one.
//------------------------------------------------------------------------------
namespace
{
// `path` or `id`; the id is resolved against the project's characters
bool ResolveTarget(LPCSTR raw, xr_string& abs, xr_string& err)
{
	xr_string path;
	if (NqUtil::ArgString(raw, "path", path) && !path.empty())
		return NqUtil::ProjectPath(path.c_str(), abs, err);

	xr_string id;
	if (!NqUtil::ArgString(raw, "id", id) || id.empty())
	{
		err = "give 'path' (a .npcasset in the project) or 'id' (a character id)";
		return false;
	}
	xr_vector<EditorCharacters::SCharacter>	all;
	xr_vector<xr_string>					problems;
	EditorCharacters::LoadAll(EditorProject::Root(), all, problems);
	for (u32 i = 0; i < all.size(); ++i)
		if (all[i].id == id)	{ abs = all[i].file; return true; }
	err = "no character with id '" + id + "' in the project";
	return false;
}

void JsonCharacter(xr_string& out, const EditorCharacters::SCharacter& c)
{
	out += "{\"id\":";
	NqUtil::JsonString(out, c.id);
	out += ",\"file\":";
	NqUtil::JsonPath(out, NqUtil::ProjectRelative(c.file.c_str()).c_str());
	out += ",\"name\":";
	NqUtil::JsonString(out, TextIn(c.name, "rus"));
	out += ",\"community\":";
	NqUtil::JsonString(out, c.community);
	out += ",\"visual\":";
	NqUtil::JsonPath(out, c.visual.c_str());
	out += ",\"base\":";
	NqUtil::JsonString(out, c.base);
	out += ",\"abstract\":";
	NqUtil::JsonBool(out, c.abstract_);
	out += ",\"supplies\":";
	NqUtil::JsonInt(out, (int)c.supplies.size());
	out += ",\"squad\":";
	NqUtil::JsonBool(out, c.squad);
	out += "}";
}
} // namespace

void EditorCharacters::McpList(LPCSTR raw, xr_string& out)
{
	(void)raw;
	if (!EditorProject::Active())	{ NqUtil::JsonError(out, "no active project"); return; }

	xr_vector<SCharacter>	all;
	xr_vector<xr_string>	problems;
	LoadResolved(EditorProject::Root(), all, problems);
	for (u32 i = 0; i < all.size(); ++i)
		Validate(all[i], EditorGameContent::Mounted(), problems);

	xr_string mod;
	ModuleId(mod);

	out = "{\"ok\":true,\"module\":";
	NqUtil::JsonString(out, mod);
	out += ",\"characters\":[";
	for (u32 i = 0; i < all.size(); ++i)
	{
		if (i)	out += ",";
		JsonCharacter(out, all[i]);
	}
	out += "],\"problems\":";
	NqUtil::JsonStringArray(out, problems);
	out += "}";
}

void EditorCharacters::McpGet(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active())	{ NqUtil::JsonError(out, "no active project"); return; }

	xr_string abs, err;
	if (!ResolveTarget(raw, abs, err))	{ NqUtil::JsonError(out, err); return; }

	SCharacter c;
	if (!Load(abs.c_str(), c, err))		{ NqUtil::JsonError(out, err); return; }

	// the answer describes what the character REALLY is, base chain and all;
	// `text` below is still the file as written
	xr_vector<SCharacter>	all;
	xr_vector<xr_string>	problems;
	LoadResolved(EditorProject::Root(), all, problems);
	problems.clear();
	for (u32 i = 0; i < all.size(); ++i)
		if (all[i].id == c.id)	{ c = all[i]; break; }
	Validate(c, EditorGameContent::Mounted(), problems);

	xr_string text;
	NqLua::ReadFile(abs.c_str(), text, err);

	out = "{\"ok\":true,\"character\":";
	JsonCharacter(out, c);
	out += ",\"text\":";
	NqUtil::JsonString(out, text);
	out += ",\"problems\":";
	NqUtil::JsonStringArray(out, problems);
	out += "}";
}

void EditorCharacters::McpWrite(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active())	{ NqUtil::JsonError(out, "no active project"); return; }

	xr_string text;
	if (!NqUtil::ArgString(raw, "text", text) || text.empty())
		{ NqUtil::JsonError(out, "missing 'text' (the whole .npcasset body)"); return; }

	// Where it goes: an explicit path, an existing character's file, or a new
	// file named after the id the text carries.
	xr_string abs, err;
	if (!ResolveTarget(raw, abs, err))
	{
		SCharacter probe;
		if (!Parse(text.c_str(), (u32)text.size(), "character", probe, err))
			{ NqUtil::JsonError(out, err); return; }
		if (probe.id.empty())
			{ NqUtil::JsonError(out, "the text has no id and no 'path' was given"); return; }
		abs = xr_string(EditorProject::Root()) + "\\characters\\" + probe.id + ".npcasset";
	}

	SCharacter c;
	if (!Parse(text.c_str(), (u32)text.size(), NqUtil::BaseName(abs.c_str()).c_str(), c, err))
		{ NqUtil::JsonError(out, err); return; }

	string_path folder;
	xr_strcpy(folder, sizeof(folder), abs.c_str());
	if (char* slash = strrchr(folder, '\\'))	*slash = 0;
	CreateDirChain(folder);

	if (!NqLua::WriteFile(abs.c_str(), text, err))	{ NqUtil::JsonError(out, err); return; }
	EditorCharacters::Invalidate();

	c.file = abs;
	// Validate what it RESOLVES to, not what the file literally says: a count of
	// 0 is how an inherited row is dropped, and a base that does not exist only
	// shows up once the chain is walked. The file is already on disk, so the
	// resolve sees this write.
	xr_vector<SCharacter>	all;
	xr_vector<xr_string>	problems;
	LoadResolved(EditorProject::Root(), all, problems);
	xr_vector<xr_string> own;
	for (u32 i = 0; i < problems.size(); ++i)
		if (problems[i].find(NqUtil::ProjectRelative(abs.c_str())) != xr_string::npos)
			own.push_back(problems[i]);
	for (u32 i = 0; i < all.size(); ++i)
		if (all[i].id == c.id)	{ c = all[i]; break; }
	Validate(c, EditorGameContent::Mounted(), own);
	problems = own;

	out = "{\"ok\":true,\"file\":";
	NqUtil::JsonPath(out, NqUtil::ProjectRelative(abs.c_str()).c_str());
	out += ",\"character\":";
	JsonCharacter(out, c);
	out += ",\"problems\":";
	NqUtil::JsonStringArray(out, problems);
	out += "}";
}

void EditorCharacters::McpDelete(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active())	{ NqUtil::JsonError(out, "no active project"); return; }

	xr_string abs, err;
	if (!ResolveTarget(raw, abs, err))	{ NqUtil::JsonError(out, err); return; }

	// The same rule every disk deletion in the editor follows: gone for good
	// backs it, because a file operation has no editor undo.
	EditorFileOps::SReport rep;
	EditorFileOps::Delete(abs.c_str(), false, rep);
	EditorCharacters::Invalidate();
	if (!rep.ok())
	{
		NqUtil::JsonError(out, rep.failures.empty() ? xr_string("delete failed") : rep.failures[0].reason);
		return;
	}
	out = "{\"ok\":true,\"deleted\":";
	NqUtil::JsonPath(out, NqUtil::ProjectRelative(abs.c_str()).c_str());
	out += "}";
}

//------------------------------------------------------------------------------
// the window
//
// The asset is text and an AI writes it directly; this is the hand path, and it
// exists so that "give my stalker a name, a coat and a rifle" does not mean
// remembering the shape of three game config formats at once.
//------------------------------------------------------------------------------
namespace
{
const char* kTitleDeleteChar = "Delete character###DeleteCharacter";

bool						s_Open			= false;
bool						s_NeedReload	= true;
xr_vector<EditorCharacters::SCharacter>	s_All;
xr_vector<xr_string>		s_Problems;
int							s_Sel			= -1;
EditorCharacters::SCharacter s_Edit;
bool						s_Dirty			= false;
bool						s_WantDelete	= false;
xr_string					s_Status;
xr_string					s_PendingOpen;

xr_vector<EditorCharacters::SSupply>	s_Inherited;	// what the base contributes
xr_string					s_InheritedCommunity;	// and the community it would give

char	s_Id[64]		= "";
char	s_Base[64]		= "";
char	s_NameRus[256]	= "";
char	s_NameEng[256]	= "";
char	s_BioRus[1024]	= "";
char	s_BioEng[1024]	= "";
char	s_Community[64]	= "";
char	s_Visual[260]	= "";
char	s_Icon[128]		= "";
char	s_Dialog[128]	= "";

// Writes one language back without losing the others: a plain string stays a
// plain string while only rus is in play, and becomes a table the moment a
// second language exists.
void TextSet(SNqValue& v, LPCSTR lang, const xr_string& text)
{
	const bool is_rus = (0 == strcmp(lang, "rus"));
	if (v.IsTable())
	{
		if (text.empty())	v.Erase(lang);
		else				v.Set(lang, SNqValue::String(text));
		if (v.keys.empty())	v = SNqValue();
		return;
	}
	if (is_rus)
	{
		v = text.empty() ? SNqValue() : SNqValue::String(text);
		return;
	}
	// a second language arrives: what was there keeps its place as rus
	const xr_string had = v.IsString() ? v.s : xr_string();
	v = SNqValue::Table();
	if (!had.empty())	v.Set("rus", SNqValue::String(had));
	if (!text.empty())	v.Set(lang, SNqValue::String(text));
	if (v.keys.empty())	v = SNqValue();
}

// { rus = "..." } and "..." mean the same thing downstream, so the simple case
// stays simple in the file: only a real second language makes it a table.
void TextCollapse(SNqValue& v)
{
	if (!v.IsTable() || v.keys.size() != 1 || v.keys[0] != "rus")	return;
	if (!v.vals[0].IsString())										return;
	v = SNqValue::String(v.vals[0].s);
}

void CopyTo(char* dst, u32 size, const xr_string& src)
{
	strncpy_s(dst, size, src.c_str(), _TRUNCATE);
}

// What the base chain gives this character, so the form can show the kit that
// is already there instead of letting the author add it a second time.
void RecomputeInherited()
{
	s_Inherited.clear();
	s_InheritedCommunity.clear();
	if (s_Edit.base.empty() || !EditorProject::Active())	return;

	xr_vector<EditorCharacters::SCharacter>	all;
	xr_vector<xr_string>					ignored;
	EditorCharacters::LoadAll(EditorProject::Root(), all, ignored);
	// ask for the BASE resolved, which is exactly what this character starts from
	EditorCharacters::Resolve(all, ignored);
	for (u32 i = 0; i < all.size(); ++i)
		if (all[i].id == s_Edit.base)
		{
			s_Inherited				= all[i].supplies;
			s_InheritedCommunity	= all[i].community;
			return;
		}
}

// A community a stalker can be of. The list of the linked game also carries the
// monsters' (their own section, plus "monster" - what every one of them is to a
// human) and the player's own, and none of those makes an NPC.
bool StalkerCommunity(const NqPickers::SEntry& e)
{
	if (e.extra == "monster_communities" || e.id == "monster")	return false;
	return e.id != "actor" && 0 != strncmp(e.id.c_str(), "actor_", 6);
}

// The communities of the LINKED game, under the names its player reads. Typed by
// hand this was a field to get right from memory, and the engine refuses a
// character of a community it does not know. A value the game does not list - a
// community the module brings itself - is kept and shown as such, never corrected.
bool CommunityCombo()
{
	const xr_vector<NqPickers::SEntry>& all = NqPickers::Index(NqPickers::tCommunity);
	if (all.empty())
	{
		// no linked game to read the list from, so the field stays a field
		const bool typed = ImGui::InputText("##community", s_Community, sizeof(s_Community));
		if (ImGui::IsItemHovered())	ImGui::SetTooltip("No linked game to list the communities of - type the id.");
		return typed;
	}

	const NqPickers::SEntry* cur = 0;
	for (u32 i = 0; i < all.size(); ++i)
		if (all[i].id == s_Community)	{ cur = &all[i]; break; }

	// unset is a value of its own: a character with a base takes the base's
	char unset[160];
	if (s_Edit.base.empty())	xr_strcpy(unset, sizeof(unset), "(not set - stalker)");
	else						sprintf_s(unset, "(from %s%s%s)", s_Edit.base.c_str(),
									s_InheritedCommunity.empty() ? "" : " - ", s_InheritedCommunity.c_str());

	char preview[256];
	if (!s_Community[0])				xr_strcpy(preview, sizeof(preview), unset);
	else if (cur && !cur->name.empty())	sprintf_s(preview, "%s  -  %s", cur->id.c_str(), cur->name.c_str());
	else								xr_strcpy(preview, sizeof(preview), s_Community);

	bool picked = false;
	if (ImGui::BeginCombo("##community", preview, ImGuiComboFlags_HeightLarge))
	{
		if (ImGui::Selectable(unset, !s_Community[0]))	{ s_Community[0] = 0; picked = true; }
		if (s_Community[0] && !cur)
		{
			ImGui::Selectable(s_Community, true);
			ImGui::SameLine();
			ImGui::TextDisabled("not in the linked game");
		}
		for (u32 i = 0; i < all.size(); ++i)
		{
			if (!StalkerCommunity(all[i]) && &all[i] != cur)	continue;
			ImGui::PushID((int)i);
			if (ImGui::Selectable(all[i].id.c_str(), &all[i] == cur))
			{
				CopyTo(s_Community, sizeof(s_Community), all[i].id);
				picked = true;
			}
			if (!all[i].name.empty())
			{
				ImGui::SameLine();
				ImGui::TextDisabled("%s", all[i].name.c_str());
			}
			ImGui::PopID();
		}
		ImGui::EndCombo();
	}
	return picked;
}

// ---- pickers -----------------------------------------------------------------
// One picker popup is open at a time, so they share the filter.
char							s_Filter[96]	= "";
EditorCharacters::TModelPreview	s_ModelPreview	= 0;

// ASCII and Cyrillic folded to lower case: what the author types is matched against
// ids and against the captions the player reads
void Fold(LPCSTR text, xr_string& out)
{
	out.clear();
	if (!text)	return;
	for (const u8* p = (const u8*)text; *p; ++p)
	{
		const u8 c = *p;
		if (c >= 'A' && c <= 'Z')	{ out += char(c + 0x20); continue; }
		if (c == 0xD0 && p[1])
		{
			const u8 d = p[1];
			if (d == 0x81)					{ out += char(0xD1); out += char(0x91); ++p; continue; }
			if (d >= 0x90 && d <= 0x9F)		{ out += char(0xD0); out += char(d + 0x20); ++p; continue; }
			if (d >= 0xA0 && d <= 0xAF)		{ out += char(0xD1); out += char(d - 0x20); ++p; continue; }
		}
		out += char(c);
	}
}

bool Matches(const NqPickers::SEntry& e, const xr_string& query)
{
	if (query.empty())	return true;
	xr_string f;
	Fold(e.id.c_str(), f);
	if (f.find(query) != xr_string::npos)	return true;
	Fold(e.name.c_str(), f);
	return f.find(query) != xr_string::npos;
}

// the entry with that id - the lists come sorted by id
const NqPickers::SEntry* FindEntry(const xr_vector<NqPickers::SEntry>& list, LPCSTR id)
{
	if (!id || !id[0])	return 0;
	const xr_string key = id;
	xr_vector<NqPickers::SEntry>::const_iterator it = std::lower_bound(list.begin(), list.end(), key,
		[](const NqPickers::SEntry& e, const xr_string& k) { return e.id < k; });
	return (it != list.end() && it->id == key) ? &*it : 0;
}

// A combo over a long list: a filter on top, only the rows in view drawn. The field
// shows "id - caption"; a value the list does not hold stays and is marked, never
// replaced - the text path may name what only the module brings.
bool ListPicker(LPCSTR label, char* buf, u32 size, const xr_vector<NqPickers::SEntry>& list, LPCSTR none, float width)
{
	const NqPickers::SEntry* cur = FindEntry(list, buf);
	xr_string preview = buf[0] ? xr_string(buf) : xr_string(none ? none : "");
	if (cur && !cur->name.empty())	preview += "  -  " + cur->name;
	const bool unknown = buf[0] && !cur && !list.empty();
	bool picked = false;
	ImGui::SetNextItemWidth(width);
	if (unknown)	ImGui::PushStyleColor(ImGuiCol_Text, ImVec4(1.f, .8f, .35f, 1.f));
	const bool open = ImGui::BeginCombo(label, preview.c_str(), ImGuiComboFlags_HeightLarge);
	if (unknown)	ImGui::PopStyleColor();
	if (unknown && !open && ImGui::IsItemHovered())
		ImGui::SetTooltip("'%s' is not in the linked game or the project - kept as written", buf);
	if (!open)	return false;

	if (ImGui::IsWindowAppearing())	{ s_Filter[0] = 0; ImGui::SetKeyboardFocusHere(); }
	ImGui::SetNextItemWidth(-1.f);
	ImGui::InputTextWithHint("##filter", "type to filter", s_Filter, sizeof(s_Filter));
	if (none && ImGui::Selectable(none, !buf[0]))	{ buf[0] = 0; picked = true; }
	xr_string query;
	Fold(s_Filter, query);
	static xr_vector<u32> shown;
	shown.clear();
	for (u32 i = 0; i < list.size(); ++i)
		if (Matches(list[i], query))	shown.push_back(i);
	ImGuiListClipper clip;
	clip.Begin((int)shown.size());
	while (clip.Step())
		for (int r = clip.DisplayStart; r < clip.DisplayEnd; ++r)
		{
			const NqPickers::SEntry& e = list[shown[r]];
			ImGui::PushID(r);
			if (ImGui::Selectable(e.id.c_str(), e.id == buf))	{ CopyTo(buf, size, e.id); picked = true; }
			if (!e.name.empty())
			{
				ImGui::SameLine();
				ImGui::TextDisabled("%s", e.name.c_str());
			}
			ImGui::PopID();
		}
	if (shown.empty())	ImGui::TextDisabled("nothing matches");
	ImGui::EndCombo();
	return picked;
}

// ---- portraits ------------------------------------------------------------------
// A portrait is a rectangle of a sheet that holds a hundred of them: every sheet is
// loaded once, the first time a face on it is shown, and kept for the session.
struct SSheet
{
	ID3D11ShaderResourceView*	view;
	u32							w, h;
	bool						tried;
	SSheet() : view(0), w(0), h(0), tried(false) {}
};
xr_map<xr_string, SSheet>	s_Sheets;
xr_string					s_SheetsRoot;

void DropSheets()
{
	for (xr_map<xr_string, SSheet>::iterator it = s_Sheets.begin(); it != s_Sheets.end(); ++it)
		if (it->second.view)	it->second.view->Release();
	s_Sheets.clear();
}

// extra = "[project:]<sheet>|x|y|w|h", the sheet below textures\ of the game or the project
bool Portrait(const NqPickers::SEntry& e, ImTextureID& tex, ImVec2& uv0, ImVec2& uv1)
{
	xr_string extra = e.extra;
	const bool project = 0 == extra.compare(0, 8, "project:");
	if (project)	extra.erase(0, 8);
	xr_vector<xr_string> part;
	NqUtil::Split(extra.c_str(), '|', part);
	if (part.size() != 5)	return false;
	SSheet& s = s_Sheets[(project ? "p|" : "g|") + part[0]];
	if (!s.tried)
	{
		s.tried = true;
		if (project)
		{
			string_path abs;
			sprintf_s(abs, "%s\\gamedata\\textures\\%s.dds", EditorProject::Root(), part[0].c_str());
			s.view = DX11TextureFromFile(abs);
		}
		else
		{
			xr_string rel = "textures\\" + part[0] + ".dds";
			xr_strlwr(&rel[0]);
			const int idx = EditorGameContent::Find(rel.c_str());
			u32 size = 0;
			u8* bytes = (idx >= 0) ? EditorGameContent::ReadBytes(idx, size) : 0;
			if (bytes)
			{
				s.view = DX11TextureFromMemory(bytes, size);
				EditorGameContent::FreeBytes(bytes);
			}
		}
		if (s.view)	DX11TextureSize(s.view, s.w, s.h);
	}
	if (!s.view || !s.w || !s.h)	return false;
	const float x = (float)atof(part[1].c_str()), y = (float)atof(part[2].c_str());
	const float w = (float)atof(part[3].c_str()), h = (float)atof(part[4].c_str());
	tex = (ImTextureID)s.view;
	uv0 = ImVec2(x / s.w, y / s.h);
	uv1 = ImVec2((x + w) / s.w, (y + h) / s.h);
	return true;
}

// The face the player sees, beside a gallery of every face the linked game and the
// project have. Picked by looking, not by remembering ui_inGame2_neutral_2_mask.
bool IconPicker(char* buf, u32 size, float em)
{
	const xr_vector<NqPickers::SEntry>& list = NqPickers::Index(NqPickers::tIcon);
	const NqPickers::SEntry* cur = FindEntry(list, buf);
	const ImVec2 face(em * 5.4f, em * 3.8f);
	ImTextureID tex = 0;
	ImVec2 uv0, uv1;
	if (cur && Portrait(*cur, tex, uv0, uv1))	ImGui::Image(tex, face, uv0, uv1);
	else
	{
		const ImVec2 at = ImGui::GetCursorScreenPos();
		ImGui::Dummy(face);
		ImGui::GetWindowDrawList()->AddRect(at, ImVec2(at.x + face.x, at.y + face.y),
			ImGui::GetColorU32(ImGuiCol_Border), 3.f);
	}
	if (ImGui::IsItemHovered())	ImGui::SetTooltip("%s", buf[0] ? buf : "no portrait");
	ImGui::SameLine();

	const bool unknown = buf[0] && !cur && !list.empty();
	if (unknown)	ImGui::PushStyleColor(ImGuiCol_Text, ImVec4(1.f, .8f, .35f, 1.f));
	ImGui::SetNextItemWidth(-1.f);
	const bool open = ImGui::BeginCombo("##icon", buf[0] ? buf : "(none)", ImGuiComboFlags_HeightLargest);
	if (unknown)	ImGui::PopStyleColor();
	if (!open)	return false;

	bool picked = false;
	if (ImGui::IsWindowAppearing())	{ s_Filter[0] = 0; ImGui::SetKeyboardFocusHere(); }
	ImGui::SetNextItemWidth(-1.f);
	ImGui::InputTextWithHint("##filter", "type to filter", s_Filter, sizeof(s_Filter));
	if (ImGui::Selectable("(none)", !buf[0]))	{ buf[0] = 0; picked = true; }
	xr_string query;
	Fold(s_Filter, query);
	static xr_vector<u32> shown;
	shown.clear();
	for (u32 i = 0; i < list.size(); ++i)
		if (Matches(list[i], query))	shown.push_back(i);

	// a gallery: as many columns as fit, only the rows in view drawn
	const ImGuiStyle& st = ImGui::GetStyle();
	const ImVec2 cell(em * 6.2f, em * 4.4f);
	const int cols = _max(1, int((ImGui::GetContentRegionAvail().x + st.ItemSpacing.x) /
								(cell.x + st.FramePadding.x * 2.f + st.ItemSpacing.x)));
	ImGuiListClipper clip;
	clip.Begin((int(shown.size()) + cols - 1) / cols, cell.y + st.FramePadding.y * 2.f + st.ItemSpacing.y);
	while (clip.Step())
		for (int r = clip.DisplayStart; r < clip.DisplayEnd; ++r)
			for (int c = 0; c < cols; ++c)
			{
				const int n = r * cols + c;
				if (n >= (int)shown.size())	break;
				const NqPickers::SEntry& e = list[shown[n]];
				if (c)	ImGui::SameLine();
				ImGui::PushID(n);
				const bool clicked = Portrait(e, tex, uv0, uv1)
					? ImGui::ImageButton("##face", tex, cell, uv0, uv1)
					: ImGui::Button(e.id.c_str(), ImVec2(cell.x + st.FramePadding.x * 2.f, cell.y + st.FramePadding.y * 2.f));
				if (clicked)	{ CopyTo(buf, size, e.id); picked = true; ImGui::CloseCurrentPopup(); }
				if (ImGui::IsItemHovered())
					ImGui::SetTooltip("%s%s", e.id.c_str(), 0 == e.extra.compare(0, 8, "project:") ? "\nthe project's" : "");
				if (e.id == buf)
					ImGui::GetWindowDrawList()->AddRect(ImGui::GetItemRectMin(), ImGui::GetItemRectMax(),
						ImGui::GetColorU32(ImGuiCol_CheckMark), 3.f, 0, 2.f);
				ImGui::PopID();
			}
	if (shown.empty())	ImGui::TextDisabled("nothing matches");
	ImGui::EndCombo();
	return picked;
}

// ---- models -------------------------------------------------------------------------
// NPC models: meshes\actors of the linked game and of the project, spelled the way the
// field wants them - below meshes\, no extension
xr_vector<NqPickers::SEntry>	s_Visuals;
xr_string						s_VisualsFor;

const xr_vector<NqPickers::SEntry>& Visuals()
{
	const xr_string key = NqUtil::Format("%s|%d", EditorProject::Active() ? EditorProject::Root() : "",
		EditorGameContent::Mounted() ? EditorGameContent::Count() : -1);
	if (key == s_VisualsFor)	return s_Visuals;
	s_VisualsFor = key;
	s_Visuals.clear();
	xr_set<xr_string> seen;
	if (EditorGameContent::Mounted())
	{
		const int total = EditorGameContent::Count();
		for (int i = 0; i < total; ++i)
		{
			const EditorGameContent::SItem* it = EditorGameContent::Get(i);
			if (!it || it->path.size() < 19 || 0 != _strnicmp(it->path.c_str(), "meshes\\actors\\", 14))	continue;
			if (0 != _stricmp(it->path.c_str() + it->path.size() - 4, ".ogf"))	continue;
			NqPickers::SEntry e;
			e.id = it->path.substr(7, it->path.size() - 7 - 4);
			if (seen.insert(e.id).second)	s_Visuals.push_back(e);
		}
	}
	if (EditorProject::Active())
	{
		string_path dir;
		sprintf_s(dir, "%s\\gamedata\\meshes\\actors", EditorProject::Root());
		xr_vector<xr_string> files;
		NqUtil::ListFiles(dir, ".ogf", files);
		const size_t cut = xr_strlen(EditorProject::Root()) + xr_strlen("\\gamedata\\meshes\\");
		for (u32 i = 0; i < files.size(); ++i)
		{
			if (files[i].size() <= cut + 4)	continue;
			NqPickers::SEntry e;
			e.id = files[i].substr(cut, files[i].size() - cut - 4);
			xr_strlwr(&e.id[0]);
			e.name = "the project's";
			if (seen.insert(e.id).second)	s_Visuals.push_back(e);
		}
	}
	std::sort(s_Visuals.begin(), s_Visuals.end(),
		[](const NqPickers::SEntry& a, const NqPickers::SEntry& b) { return a.id < b.id; });
	return s_Visuals;
}

// ---- ranks --------------------------------------------------------------------------
// The linked game's ranks in its own words, lowest first, and the number kept beside
// them: Dead Air counts to 14000, not the 1000 of the old games.
bool RankPicker(int& rank, bool& has_rank)
{
	const xr_vector<NqPickers::SEntry>& all = NqPickers::Index(NqPickers::tRank);
	xr_vector<const NqPickers::SEntry*> ranks;
	for (u32 i = 0; i < all.size(); ++i)	ranks.push_back(&all[i]);
	std::sort(ranks.begin(), ranks.end(), [](const NqPickers::SEntry* a, const NqPickers::SEntry* b)
		{ return atoi(a->extra.c_str()) < atoi(b->extra.c_str()); });
	if (ranks.empty())	return false;

	int cur = -1;
	for (u32 i = 0; i < ranks.size(); ++i)
		if (rank >= atoi(ranks[i]->extra.c_str()))	cur = (int)i;
	xr_string preview = !has_rank ? xr_string("(not set)")
					  : cur < 0 ? xr_string("?")
					  : (ranks[cur]->name.empty() ? ranks[cur]->id : ranks[cur]->name + "  (" + ranks[cur]->id + ")");
	bool picked = false;
	if (ImGui::BeginCombo("##rankname", preview.c_str()))
	{
		for (u32 i = 0; i < ranks.size(); ++i)
		{
			const int from = atoi(ranks[i]->extra.c_str());
			const int to = (i + 1 < ranks.size()) ? atoi(ranks[i + 1]->extra.c_str()) - 1 : -1;
			const xr_string item = (ranks[i]->name.empty() ? ranks[i]->id : ranks[i]->name + "  (" + ranks[i]->id + ")") +
				(to >= 0 ? NqUtil::Format("   %d-%d", from, to) : NqUtil::Format("   %d+", from));
			if (ImGui::Selectable(item.c_str(), has_rank && (int)i == cur))
			{
				// the number stays when it already is that rank
				if (!has_rank || (int)i != cur)	rank = from;
				has_rank = true;
				picked = true;
			}
		}
		ImGui::EndCombo();
	}
	return picked;
}

// Another character of the project to start from: picked from the ones that exist,
// because a base that does not is E111 and nothing else.
bool BaseCombo()
{
	bool known = !s_Base[0];
	for (u32 i = 0; i < s_All.size() && !known; ++i)
		known = ((int)i != s_Sel && s_All[i].id == s_Base);

	bool picked = false;
	if (ImGui::BeginCombo("##base", s_Base[0] ? s_Base : "(none)", ImGuiComboFlags_HeightLarge))
	{
		if (ImGui::Selectable("(none)", !s_Base[0]))	{ s_Base[0] = 0; picked = true; }
		if (!known)
		{
			ImGui::Selectable(s_Base, true);
			ImGui::SameLine();
			ImGui::TextDisabled("not in the project");
		}
		for (u32 i = 0; i < s_All.size(); ++i)
		{
			// not itself: a character that starts from itself is E112
			if ((int)i == s_Sel)	continue;
			ImGui::PushID((int)i);
			if (ImGui::Selectable(s_All[i].id.c_str(), s_All[i].id == s_Base))
			{
				CopyTo(s_Base, sizeof(s_Base), s_All[i].id);
				picked = true;
			}
			if (s_All[i].abstract_)
			{
				ImGui::SameLine();
				ImGui::TextDisabled("template");
			}
			ImGui::PopID();
		}
		ImGui::EndCombo();
	}
	return picked;
}

void PullToBuffers()
{
	CopyTo(s_Id, sizeof(s_Id), s_Edit.id);
	CopyTo(s_Base, sizeof(s_Base), s_Edit.base);
	CopyTo(s_NameRus, sizeof(s_NameRus), TextRaw(s_Edit.name, "rus"));
	CopyTo(s_NameEng, sizeof(s_NameEng), TextRaw(s_Edit.name, "eng"));
	CopyTo(s_BioRus, sizeof(s_BioRus), TextRaw(s_Edit.bio, "rus"));
	CopyTo(s_BioEng, sizeof(s_BioEng), TextRaw(s_Edit.bio, "eng"));
	CopyTo(s_Community, sizeof(s_Community), s_Edit.community);
	CopyTo(s_Visual, sizeof(s_Visual), s_Edit.visual);
	CopyTo(s_Icon, sizeof(s_Icon), s_Edit.icon);
	CopyTo(s_Dialog, sizeof(s_Dialog), s_Edit.start_dialog);
}

void SelectIndex(int index)
{
	s_Sel = index;
	if (index >= 0 && index < (int)s_All.size())	s_Edit = s_All[index];
	else										s_Edit.Reset();
	s_Dirty = false;
	PullToBuffers();
	RecomputeInherited();
}

void ReloadAll()
{
	const xr_string keep = (s_Sel >= 0 && s_Sel < (int)s_All.size()) ? s_All[s_Sel].id : xr_string();
	s_Problems.clear();
	EditorCharacters::LoadAll(EditorProject::Root(), s_All, s_Problems);

	int sel = s_All.empty() ? -1 : 0;
	for (u32 i = 0; i < s_All.size(); ++i)
		if (s_All[i].id == keep)	sel = (int)i;
	SelectIndex(sel);
	s_NeedReload = false;
}

bool SaveEdited(xr_string& err)
{
	if (s_Edit.id.empty())	{ err = "the character needs an id"; return false; }
	if (s_Edit.file.empty())
	{
		s_Edit.file = xr_string(EditorProject::Root()) + "\\characters\\" + s_Edit.id + ".npcasset";
		string_path folder;
		xr_strcpy(folder, sizeof(folder), s_Edit.file.c_str());
		if (char* slash = strrchr(folder, '\\'))	*slash = 0;
		CreateDirChain(folder);
	}
	if (!EditorCharacters::Save(s_Edit.file.c_str(), s_Edit, err))	return false;
	s_Dirty			= false;
	s_NeedReload	= true;
	s_Status		= "Saved " + NqUtil::ProjectRelative(s_Edit.file.c_str());
	return true;
}

void DrawDeleteConfirm()
{
	if (!s_WantDelete)	return;
	if (!ImGui::BeginPopupModal(kTitleDeleteChar, &s_WantDelete, ImGuiWindowFlags_AlwaysAutoResize, true))
		return;

	// Names what disappears, not "are you sure": the file itself goes to the
	// disk, and the module loses the character and anything aimed at it.
	ImGui::Text("Delete the character \"%s\"?", s_Edit.id.c_str());
	ImGui::TextDisabled("%s", NqUtil::ProjectRelative(s_Edit.file.c_str()).c_str());
	ImGui::Spacing();
	ImGui::TextWrapped("The file is deleted for good. The next build takes the character out"
					   " of the module, and a quest that spawns it will have nothing to spawn.");
	ImGui::Spacing();

	if (ImGui::Button("Cancel") || ImGui::IsKeyPressed(ImGuiKey_Escape) || ImGui::IsKeyPressed(ImGuiKey_Enter))
	{
		s_WantDelete = false;
		ImGui::CloseCurrentPopup();
	}
	ImGui::SameLine();
	if (ImGui::Button("Delete"))
	{
		EditorFileOps::SReport rep;
		EditorFileOps::Delete(s_Edit.file.c_str(), false, rep);
		EditorCharacters::Invalidate();
		s_Status = rep.ok()
			? xr_string("Deleted ") + NqUtil::ProjectRelative(s_Edit.file.c_str())
			: xr_string("Delete failed");
		s_WantDelete	= false;
		s_NeedReload	= true;
		ImGui::CloseCurrentPopup();
	}
	ImGui::EndPopup();
}
} // namespace

bool EditorCharacters::IsOpen()	{ return s_Open; }

void EditorCharacters::Show()
{
	s_Open			= true;
	s_NeedReload	= true;
}

void EditorCharacters::Open(LPCSTR path)
{
	s_Open			= true;
	s_NeedReload	= true;
	s_PendingOpen	= path ? path : "";
}

void EditorCharacters::SetModelPreview(TModelPreview fn)	{ s_ModelPreview = fn; }

void EditorCharacters::DrawUI()
{
	DrawDeleteConfirm();
	// portraits of one project are not the next one's; dropped before anything of
	// this frame can reference them
	const xr_string root = EditorProject::Active() ? EditorProject::Root() : "";
	if (root != s_SheetsRoot)
	{
		DropSheets();
		s_SheetsRoot = root;
	}
	if (!s_Open)	return;

	if (!EditorProject::Active())
	{
		s_All.clear();
		s_Sel = -1;
	}
	else if (s_NeedReload)
	{
		ReloadAll();
		if (!s_PendingOpen.empty())
		{
			for (u32 i = 0; i < s_All.size(); ++i)
				if (0 == _stricmp(s_All[i].file.c_str(), s_PendingOpen.c_str()))	SelectIndex((int)i);
			s_PendingOpen.clear();
		}
	}

	const float em = ImGui::GetFontSize();
	ImGui::SetNextWindowSize(ImVec2(em * 48.f, em * 38.f), ImGuiCond_FirstUseEver);
	if (!ImGui::Begin("Characters", &s_Open))
	{
		ImGui::End();
		return;
	}

	if (!EditorProject::Active())
	{
		ImGui::TextDisabled("Open a project first.");
		ImGui::End();
		return;
	}

	xr_string mod;
	ModuleId(mod);

	// ---- toolbar ----------------------------------------------------------
	if (ImGui::Button("New"))
	{
		SCharacter c;
		c.id = "new_character";
		for (int n = 2; n < 1000; ++n)
		{
			bool taken = false;
			for (u32 i = 0; i < s_All.size(); ++i)
				if (s_All[i].id == c.id)	taken = true;
			if (!taken)	break;
			c.id = NqUtil::Format("new_character_%d", n);
		}
		c.community	= "stalker";
		c.name		= SNqValue::String("New character");
		c.visual	= "actors\\stalker_neutral\\stalker_neutral_1";
		c.icon		= "ui_inGame2_neutral_2";
		c.has_rank	= true;
		c.rank		= 300;
		s_Edit		= c;
		s_Sel		= -1;
		s_Dirty		= true;
		PullToBuffers();
	}
	ImGui::SameLine();
	ImGui::BeginDisabled(!s_Dirty);
	if (ImGui::Button("Save"))
	{
		xr_string err;
		if (!SaveEdited(err))	s_Status = "Save failed: " + err;
	}
	ImGui::EndDisabled();
	ImGui::SameLine();
	if (ImGui::Button("Reload"))
	{
		s_NeedReload = true;
		// the project's own dialogs, portraits and models may have changed on disk
		NqPickers::Invalidate();
		s_VisualsFor.clear();
		DropSheets();
	}
	ImGui::SameLine();
	ImGui::BeginDisabled(s_Edit.file.empty());
	if (ImGui::Button("Delete..."))	s_WantDelete = true;
	ImGui::EndDisabled();
	ImGui::SameLine();
	ImGui::TextDisabled("module: %s", mod.empty() ? "(no id)" : mod.c_str());

	ImGui::Separator();

	// ---- list + form ------------------------------------------------------
	ImGui::BeginChild("##charlist", ImVec2(em * 12.f, -em * 4.6f), ImGuiChildFlags_Borders);
	for (u32 i = 0; i < s_All.size(); ++i)
	{
		const bool selected = (s_Sel == (int)i);
		if (ImGui::Selectable(s_All[i].id.c_str(), selected))
			SelectIndex((int)i);
	}
	if (s_All.empty())	ImGui::TextDisabled("no characters");
	ImGui::EndChild();

	ImGui::SameLine();
	ImGui::BeginChild("##charform", ImVec2(0, -em * 4.6f));

	const float label_w = em * 8.f;
	bool changed = false;

	ImGui::PushItemWidth(-1.f);

	ImGui::Text("Id");			ImGui::SameLine(label_w);
	if (ImGui::InputText("##id", s_Id, sizeof(s_Id)))			changed = true;
	if (ImGui::IsItemHovered())
		ImGui::SetTooltip("a-z, 0-9 and _. The game knows the character as %s_<id>.",
			mod.empty() ? "<module>" : mod.c_str());

	ImGui::Text("Base");		ImGui::SameLine(label_w);
	if (BaseCombo())
	{
		changed = true;
		s_Edit.base = s_Base;
		RecomputeInherited();
	}
	if (ImGui::IsItemHovered())
		ImGui::SetTooltip("Another character to start from: everything this one does not set\n"
						  "comes from there, and the starting kits are merged. Leave it empty\n"
						  "for a character that stands on its own.");

	ImGui::Text("Template");	ImGui::SameLine(label_w);
	if (ImGui::Checkbox("##abstract", &s_Edit.abstract_))	changed = true;
	if (ImGui::IsItemHovered())
		ImGui::SetTooltip("A template only: never becomes a character in the game, so it needs\n"
						  "no name and no model. Use it to hold a kit several characters share.");

	// Both languages on screen at once. A dropdown that swaps one field cannot
	// show what the other holds, and the text a character carries is exactly the
	// thing an author wants to compare side by side.
	ImGui::Text("Name");		ImGui::SameLine(label_w);
	ImGui::TextDisabled("rus");	ImGui::SameLine();
	ImGui::SetNextItemWidth(-1.f);
	if (ImGui::InputText("##name_rus", s_NameRus, sizeof(s_NameRus)))	changed = true;
	if (ImGui::IsItemHovered())	ImGui::SetTooltip("What the player reads above them and in dialog.");
	ImGui::Dummy(ImVec2(0.f, 0.f));	ImGui::SameLine(label_w);
	ImGui::TextDisabled("eng");	ImGui::SameLine();
	ImGui::SetNextItemWidth(-1.f);
	if (ImGui::InputText("##name_eng", s_NameEng, sizeof(s_NameEng)))	changed = true;
	if (ImGui::IsItemHovered())
		ImGui::SetTooltip("Left empty, an English install reads the Russian name.");

	ImGui::Text("Bio");			ImGui::SameLine(label_w);
	ImGui::TextDisabled("rus");	ImGui::SameLine();
	if (ImGui::InputTextMultiline("##bio_rus", s_BioRus, sizeof(s_BioRus), ImVec2(-1.f, em * 2.6f)))
		changed = true;
	ImGui::Dummy(ImVec2(0.f, 0.f));	ImGui::SameLine(label_w);
	ImGui::TextDisabled("eng");	ImGui::SameLine();
	if (ImGui::InputTextMultiline("##bio_eng", s_BioEng, sizeof(s_BioEng), ImVec2(-1.f, em * 2.6f)))
		changed = true;

	ImGui::Text("Community");	ImGui::SameLine(label_w);
	if (CommunityCombo())	changed = true;
	if (ImGui::IsItemHovered())
		ImGui::SetTooltip("The communities of the linked game. Not set: the base's, or stalker without one.");

	ImGui::Text("Visual");		ImGui::SameLine(label_w);
	{
		const ImGuiStyle& st = ImGui::GetStyle();
		const float button = ImGui::CalcTextSize("Preview").x + st.FramePadding.x * 2.f + st.ItemSpacing.x;
		if (ListPicker("##visual", s_Visual, sizeof(s_Visual), Visuals(),
				s_Edit.base.empty() ? "(not set)" : "(from the base)", -button))
			changed = true;
		if (ImGui::IsItemHovered())
			ImGui::SetTooltip("The model it spawns with and keeps: meshes\\actors of the linked game and the project.");
		ImGui::SameLine();
		ImGui::BeginDisabled(!s_ModelPreview || !s_Visual[0]);
		if (ImGui::Button("Preview"))	s_ModelPreview(s_Visual);
		ImGui::EndDisabled();
	}

	ImGui::Text("Icon");		ImGui::SameLine(label_w);
	if (IconPicker(s_Icon, sizeof(s_Icon), em))	changed = true;

	ImGui::Text("Rank");		ImGui::SameLine(label_w);
	ImGui::PushItemWidth(em * 14.f);
	if (RankPicker(s_Edit.rank, s_Edit.has_rank))	changed = true;
	ImGui::PopItemWidth();
	ImGui::SameLine();
	ImGui::PushItemWidth(em * 6.f);
	if (ImGui::InputInt("##rank", &s_Edit.rank, 0, 0))	{ changed = true; s_Edit.has_rank = true; }
	ImGui::PopItemWidth();
	if (ImGui::IsItemHovered())	ImGui::SetTooltip("The exact value; the name beside it follows the linked game's ranks.");

	ImGui::Text("Money");		ImGui::SameLine(label_w);
	if (ImGui::Checkbox("##hasmoney", &s_Edit.has_money))	changed = true;
	if (s_Edit.has_money)
	{
		ImGui::SameLine();
		ImGui::PushItemWidth(em * 6.f);
		if (ImGui::InputInt("##moneymin", &s_Edit.money_min, 0, 0))	changed = true;
		ImGui::SameLine();
		ImGui::TextDisabled("to");
		ImGui::SameLine();
		if (ImGui::InputInt("##moneymax", &s_Edit.money_max, 0, 0))	changed = true;
		ImGui::PopItemWidth();
		ImGui::SameLine();
		if (ImGui::Checkbox("infinite", &s_Edit.money_infinite))	changed = true;
	}

	ImGui::Text("Squad");		ImGui::SameLine(label_w);
	if (ImGui::Checkbox("##squad", &s_Edit.squad))	changed = true;
	if (ImGui::IsItemHovered())
		ImGui::SetTooltip("Also emit a one-man squad section, so a quest can create the\n"
						  "character with spawn.squad as well as with spawn.object.");

	ImGui::Text("Dialog");		ImGui::SameLine(label_w);
	if (ListPicker("##dialog", s_Dialog, sizeof(s_Dialog), NqPickers::Index(NqPickers::tDialog), "(none)", -1.f))
		changed = true;
	if (ImGui::IsItemHovered())
		ImGui::SetTooltip("The dialog it opens with (start_dialog). Captioned with its first line.");

	ImGui::PopItemWidth();

	// ---- the kit ----------------------------------------------------------
	ImGui::Spacing();
	ImGui::Text("Starting kit");
	ImGui::SameLine();
	if (ImGui::SmallButton("Add item"))
	{
		s_Edit.supplies.push_back(SSupply());
		changed = true;
	}
	// what the base already gives: shown, not editable, and not repeated below
	const xr_vector<NqPickers::SEntry>& items = NqPickers::Index(NqPickers::tItem);
	for (u32 i = 0; i < s_Inherited.size(); ++i)
	{
		const NqPickers::SEntry* it = FindEntry(items, s_Inherited[i].section.c_str());
		ImGui::TextDisabled("%s%s%s x%d", s_Inherited[i].section.c_str(), (it && !it->name.empty()) ? "  -  " : "",
			(it && !it->name.empty()) ? it->name.c_str() : "", s_Inherited[i].count);
		if (ImGui::IsItemHovered())
			ImGui::SetTooltip("from %s - add a row with the same section to change the count,\n"
							  "or a count of 0 to take it out", s_Edit.base.c_str());
	}

	int remove = -1;
	for (u32 i = 0; i < s_Edit.supplies.size(); ++i)
	{
		ImGui::PushID((int)i);
		char section[128];
		CopyTo(section, sizeof(section), s_Edit.supplies[i].section);
		if (ListPicker("##sect", section, sizeof(section), items, 0, em * 22.f))
		{
			s_Edit.supplies[i].section = section;
			changed = true;
		}
		ImGui::SameLine();
		ImGui::PushItemWidth(em * 5.f);
		if (ImGui::InputInt("##count", &s_Edit.supplies[i].count, 0, 0))	changed = true;
		// 0 stays: with a base it means "not this one". Below zero never meant
		// anything.
		if (s_Edit.supplies[i].count < 0)	s_Edit.supplies[i].count = 0;
		if (s_Edit.supplies[i].count == 0 && s_Inherited.empty())
			s_Edit.supplies[i].count = 1;
		ImGui::PopItemWidth();
		ImGui::SameLine();
		if (ImGui::SmallButton("x"))	remove = (int)i;
		ImGui::PopID();
	}
	if (remove >= 0)
	{
		s_Edit.supplies.erase(s_Edit.supplies.begin() + remove);
		changed = true;
	}

	ImGui::EndChild();

	// ---- what is wrong with it --------------------------------------------
	ImGui::BeginChild("##charproblems", ImVec2(0, 0), ImGuiChildFlags_Borders);
	if (!s_Status.empty())	ImGui::TextDisabled("%s", s_Status.c_str());
	xr_vector<xr_string> own;
	Validate(s_Edit, EditorGameContent::Mounted(), own);
	for (u32 i = 0; i < own.size(); ++i)
	{
		const bool fatal = own[i][0] == 'E';
		ImGui::TextColored(fatal ? ImVec4(1.f, .45f, .4f, 1.f) : ImVec4(1.f, .8f, .35f, 1.f),
			"%s", own[i].c_str());
	}
	if (own.empty())
		ImGui::TextDisabled("Build the mod and the character is in the game.");
	ImGui::EndChild();

	if (changed)
	{
		s_Edit.id			= s_Id;
		s_Edit.base			= s_Base;
		s_Edit.community	= s_Community;
		s_Edit.visual		= s_Visual;
		s_Edit.icon			= s_Icon;
		s_Edit.start_dialog	= s_Dialog;
		TextSet(s_Edit.name, "rus", s_NameRus);
		TextSet(s_Edit.name, "eng", s_NameEng);
		TextSet(s_Edit.bio, "rus", s_BioRus);
		TextSet(s_Edit.bio, "eng", s_BioEng);
		TextCollapse(s_Edit.name);
		TextCollapse(s_Edit.bio);
		if (s_Edit.money_max < s_Edit.money_min)	s_Edit.money_max = s_Edit.money_min;
		s_Dirty = true;
	}

	ImGui::End();
}
