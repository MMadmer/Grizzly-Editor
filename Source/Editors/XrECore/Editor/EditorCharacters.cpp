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

// A text is a plain string or a table keyed by language; both shapes are walked the same way.
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

LPCSTR ModuleId(xr_string& storage)
{
	EditorMod::SManifest m;
	if (EditorProject::Root() && EditorMod::Load(EditorProject::Root(), m))
		storage = m.id;
	return storage.c_str();
}

// The bone names of a skeletal model (OGF_S_BONE_NAMES); false when it carries none.
bool OgfBoneNames(const u8* data, u32 size, xr_vector<xr_string>& out)
{
	for (u32 pos = 0; pos + 8 <= size;)
	{
		const u32 id = *(const u32*)(data + pos), len = *(const u32*)(data + pos + 4);
		const u32 at = pos + 8, end = at + len;
		if (end > size || end < at)
			return false;
		// a count, then per bone its name, its parent's name and an obb of 15 floats
		if ((id & 0x7fffffff) == 13)
		{
			if (len < 4)
				return false;
			const u32 count = *(const u32*)(data + at);
			u32 p = at + 4;
			for (u32 i = 0; i < count; ++i)
			{
				const u8* name_end = p < end ? (const u8*)memchr(data + p, 0, end - p) : 0;
				if (!name_end)
					return false;
				out.push_back(xr_string((LPCSTR)(data + p)));
				p = u32(name_end - data) + 1;
				const u8* parent_end = p < end ? (const u8*)memchr(data + p, 0, end - p) : 0;
				if (!parent_end)
					return false;
				p = u32(parent_end - data) + 1 + 60;
				if (p > end)
					return false;
			}
			return true;
		}
		pos = end;
	}
	return false;
}

// The bones the damage section of a stalker names: the engine stops on the first one a model
// lacks (CDamageManager::load_section). Every spawn section the build writes is ":stalker".
void StalkerDamageBones(xr_vector<xr_string>& out)
{
	if (!pSettings || !pSettings->line_exist("stalker", "damage"))
		return;
	LPCSTR sect = pSettings->r_string("stalker", "damage");
	if (!pSettings->section_exist(sect))
		return;
	for (const auto& item : pSettings->r_section(sect).Data)
		if (xr_strcmp(item.first.c_str(), "default"))
			out.push_back(item.first.c_str());
}

// A model on a skeleton of its own - Clear Sky's Forester has no eyes and no jaw - passes every
// other check and stops the game the moment the NPC comes online. Read once per model.
void CheckSkeleton(const xr_string& rel, const xr_string& where, xr_vector<xr_string>& problems)
{
	static xr_map<xr_string, xr_vector<xr_string>> s_Bones;
	static xr_string s_BonesOf;
	if (s_BonesOf != EditorGameContent::MountedRoot())
	{
		s_Bones.clear();
		s_BonesOf = EditorGameContent::MountedRoot();
	}
	xr_string key = rel;
	xr_strlwr(&key[0]);
	auto it = s_Bones.find(key);
	if (it == s_Bones.end())
	{
		xr_vector<xr_string> bones;
		u32 size = 0;
		u8* data = EditorGameContent::ReadBytes(EditorGameContent::Find(rel.c_str()), size);
		if (data && !OgfBoneNames(data, size, bones))
			bones.clear();
		EditorGameContent::FreeBytes(data);
		it = s_Bones.emplace(key, bones).first;
	}
	// not skeletal or unreadable: nothing to tell
	if (it->second.empty())
		return;
	xr_vector<xr_string> need;
	StalkerDamageBones(need);
	xr_string missing;
	for (const xr_string& bone : need)
	{
		bool found = false;
		for (const xr_string& have : it->second)
			if (0 == _stricmp(have.c_str(), bone.c_str()))
			{
				found = true;
				break;
			}
		if (!found)
			missing += (missing.empty() ? "" : ", ") + bone;
	}
	if (!missing.empty())
		problems.push_back("E123 " + where + ": the model has no bone " + missing +
			" - an NPC's damage names them and the game stops the moment it comes online; pick a model on the stalker skeleton");
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
	trade.Reset();
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

	// trade = false says "not a trader" over whatever the base sells; a table is a shop
	if (const SNqValue* t = v.Get("trade"))
	{
		if (t->IsBool() && !t->b)
		{
			c.trade.declared	= true;
			c.trade.off			= true;
		}
		else if (t->IsTable())
		{
			c.trade.declared = true;
			auto names = [](const SNqValue* list, xr_vector<xr_string>& out)
			{
				if (!list) return;
				if (list->IsString())	{ if (!list->s.empty()) out.push_back(list->s); return; }
				for (u32 i = 0; i < list->arr.size(); ++i)
					if (list->arr[i].IsString() && !list->arr[i].s.empty()) out.push_back(list->arr[i].s);
			};
			names(t->Get("sells"), c.trade.sells);
			names(t->Get("buys"), c.trade.buys);
			c.trade.markup	= float(t->GetNumber("markup", 1.0));
			c.trade.pays	= float(t->GetNumber("pays", 0.3));
			c.trade.restock	= int(t->GetNumber("restock", 24.0));
			if (const SNqValue* s = t->Get("stock"))
				for (u32 i = 0; i < s->arr.size(); ++i)
				{
					const SNqValue& row = s->arr[i];
					STradeRow r;
					if (row.IsString())			r.section = row.s;
					else if (row.IsTable())
					{
						r.section	= row.GetString("section");
						r.count		= int(row.GetNumber("count", 1.0));
						r.chance	= float(row.GetNumber("chance", 1.0));
					}
					c.trade.stock.push_back(r);
				}
		}
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
	s += "-- A character of this module. The game reads this file as it is - see docs/MOD_QUICKSTART.md.\n";
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
	if (c.trade.declared && c.trade.off)
		s += "\ttrade     = false,\n";
	else if (c.trade.declared)
	{
		auto names = [](const xr_vector<xr_string>& list)
		{
			xr_string out = "{";
			for (u32 i = 0; i < list.size(); ++i)
				out += (i ? ", " : " ") + LuaString(list[i]);
			out += list.empty() ? "}" : " }";
			return out;
		};
		s += "\ttrade     = {\n";
		s += "\t\tsells   = " + names(c.trade.sells) + ",\n";
		s += "\t\tbuys    = " + names(c.trade.buys) + ",\n";
		s += NqUtil::Format("\t\tmarkup  = %g,\n", c.trade.markup);
		s += NqUtil::Format("\t\tpays    = %g,\n", c.trade.pays);
		s += NqUtil::Format("\t\trestock = %d,\n", c.trade.restock);
		if (!c.trade.stock.empty())
		{
			s += "\t\tstock   = {\n";
			for (u32 i = 0; i < c.trade.stock.size(); ++i)
			{
				const STradeRow& r = c.trade.stock[i];
				s += "\t\t\t{ section = " + LuaString(r.section);
				if (r.count != 1)		s += NqUtil::Format(", count = %d", r.count);
				if (r.chance != 1.f)	s += NqUtil::Format(", chance = %g", r.chance);
				s += " },\n";
			}
			s += "\t\t},\n";
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
	// a stable order: the list reads the same every time
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
	// a shop is one thing: a character that says anything about trade says all of it
	if (src.trade.declared)			dst.trade = src.trade;

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
		else
			CheckSkeleton(rel, where, problems);
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

	if (c.trade.Trader())
	{
		const STrade& t = c.trade;
		for (u32 i = 0; i < t.stock.size(); ++i)
		{
			const STradeRow& r = t.stock[i];
			if (r.section.empty())
				problems.push_back("E118 " + where + ": a stock row without an item");
			else if (r.count < 1)
				problems.push_back("E118 " + where + ": stock '" + r.section + "' asks for less than one item");
			else if (!(r.chance > 0.f && r.chance <= 1.f))
				problems.push_back("E118 " + where + ": stock '" + r.section + "' has a chance outside 0..1");
			for (u32 j = 0; j < i; ++j)
				if (!r.section.empty() && 0 == _stricmp(t.stock[j].section.c_str(), r.section.c_str()))
					problems.push_back("E118 " + where + ": '" + r.section + "' is in the stock twice - one row per item");
		}
		if (!(t.markup > 0.f))
			problems.push_back("E119 " + where + ": the markup must be above zero - it is what the trader asks of an item's cost");
		if (t.pays < 0.f)
			problems.push_back("E119 " + where + ": the trader cannot pay less than nothing");
		if (t.restock < 1)
			problems.push_back("E119 " + where + ": the stock comes back after at least one game hour");
		if (linked_game)
		{
			const xr_vector<xr_string>& cats = EditorCharacters::TradeCategories();
			auto check = [&](const xr_vector<xr_string>& list, LPCSTR what)
			{
				if (cats.empty()) return;
				for (u32 i = 0; i < list.size(); ++i)
				{
					bool known = false;
					for (u32 k = 0; k < cats.size(); ++k) if (cats[k] == list[i]) known = true;
					if (!known)
						problems.push_back("W120 " + where + ": the linked game has no trade category '" + list[i] +
							"' the trader " + what + " - the trade presets name weapon, drugs, food and the like");
				}
			};
			check(t.sells, "sells");
			check(t.buys, "buys");
			if (NqPickers::Available())
				for (u32 i = 0; i < t.stock.size(); ++i)
					if (!t.stock[i].section.empty() && !NqPickers::Known(NqPickers::tItem, t.stock[i].section.c_str()))
						problems.push_back("W121 " + where + ": the linked game has no item '" + t.stock[i].section +
							"' for the stock - right only when the module brings that section");
		}
		if (t.stock.empty() && t.sells.empty())
			problems.push_back("W122 " + where + ": the trader has nothing to sell - give it a stock or a category");
	}
}

void EditorCharacters::ValidateAll(LPCSTR project_root, bool linked_game, xr_vector<xr_string>& problems)
{
	xr_vector<SCharacter> all;
	LoadResolved(project_root, all, problems);
	for (u32 i = 0; i < all.size(); ++i)
		Validate(all[i], linked_game, problems);
}

//------------------------------------------------------------------------------
// the build: nothing is made here, the game reads the assets as they are. What is left is
// saying no to a module whose characters the game could not use.
//------------------------------------------------------------------------------
bool EditorCharacters::CheckForBuild(LPCSTR project_root, xr_string& err)
{
	if (!project_root || !project_root[0])	return true;

	xr_vector<SCharacter>	all;
	xr_vector<xr_string>	problems;
	LoadResolved(project_root, all, problems);
	// A character that does not parse must stop the build: shipping half a
	// roster produces a module whose quests point at NPCs that do not exist.
	for (u32 i = 0; i < all.size(); ++i)
		if (!all[i].abstract_)
			Validate(all[i], false, problems);
	for (u32 i = 0; i < problems.size(); ++i)
		if ('E' == problems[i][0])
		{
			err = problems[i];
			return false;
		}
	return true;
}

//------------------------------------------------------------------------------
// trade: the linked game's categories
//------------------------------------------------------------------------------
namespace
{
xr_vector<xr_string>	s_PresetNames;
xr_string				s_PresetsFor;
bool					s_PresetsRead = false;

bool ReadGameText(LPCSTR rel, xr_string& text)
{
	const int idx = EditorGameContent::Find(rel);
	if (idx < 0) return false;
	u32 size = 0;
	u8* data = EditorGameContent::ReadBytes(idx, size);
	if (!data) return false;
	text.assign((const char*)data, size);
	EditorGameContent::FreeBytes(data);
	return true;
}

// Every section of trade_presets.ltx that lists items is a category; a section that
// only inherits ("[medic]:unused,trash,...") is a recipe made of them, not one itself.
void ReadPresets()
{
	xr_string key = EditorProject::GameLinked() ? xr_string(EditorProject::GameRoot()) : xr_string();
	if (s_PresetsRead && s_PresetsFor == key) return;
	s_PresetsRead = true;
	s_PresetsFor = key;
	s_PresetNames.clear();
	xr_string err, text;
	if (key.empty() || !EditorGameContent::EnsureMounted(err)) return;
	if (!ReadGameText("configs\\misc\\trade\\trade_presets.ltx", text)) return;

	xr_string current;
	bool listed = false;
	auto flush = [&]()
	{
		if (listed && current != "unused") s_PresetNames.push_back(current);
		listed = false;
	};
	size_t pos = 0;
	while (pos < text.size())
	{
		size_t eol = text.find('\n', pos);
		if (eol == xr_string::npos) eol = text.size();
		xr_string line = NqUtil::Trim(text.substr(pos, eol - pos));
		pos = eol + 1;
		const size_t sc = line.find(';');
		if (sc != xr_string::npos) line = NqUtil::Trim(line.substr(0, sc));
		if (line.empty() || line[0] == '#') continue;
		if (line[0] == '[')
		{
			flush();
			const size_t close = line.find(']');
			const bool derived = close != xr_string::npos && line.find(':', close) != xr_string::npos;
			current = derived ? xr_string() : line.substr(1, close == xr_string::npos ? xr_string::npos : close - 1);
			continue;
		}
		if (!current.empty()) listed = true;
	}
	flush();
}
} // namespace

const xr_vector<xr_string>& EditorCharacters::TradeCategories()
{
	ReadPresets();
	return s_PresetNames;
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
	out += ",\"trader\":";
	NqUtil::JsonBool(out, c.trade.Trader());
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
// the Characters tab
//
// The asset is text and an AI writes it directly; this is the hand path, and it
// exists so that "give my stalker a name, a coat and a rifle" does not mean
// remembering the shape of three game config formats at once.
//------------------------------------------------------------------------------
namespace
{
using Token = GrizzlyTheme::ColorToken;

bool						s_Open			= false;
bool						s_NeedReload	= true;
xr_vector<EditorCharacters::SCharacter>	s_All;
xr_vector<xr_string>		s_Problems;		// files that did not load
int							s_Sel			= -1;
EditorCharacters::SCharacter s_Edit;
bool						s_Dirty			= false;
xr_string					s_Status;
xr_string					s_PendingOpen;
char						s_ListFilter[64]	= "";

xr_vector<EditorCharacters::SSupply>	s_Inherited;	// what the base contributes
xr_string					s_InheritedCommunity;	// and the community it would give
struct SMoney { bool has = false; int min = 0, max = 0; bool infinite = false; };
SMoney						s_InheritedMoney;		// and the money it would give
EditorCharacters::STrade	s_InheritedTrade;		// and the trade

// what is wrong with the edited character, worked out when it changes - never per frame
xr_vector<xr_string>		s_Own;
bool						s_OwnStale		= true;
bool						s_OwnMounted	= false;

// the module id, read when mod.ltx changes
xr_string					s_ModuleId;
u32							s_ModuleGen		= u32(-1);

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

const xr_string& CachedModuleId()
{
	if (s_ModuleGen != EditorMod::ManifestGeneration())
	{
		s_ModuleGen = EditorMod::ManifestGeneration();
		s_ModuleId.clear();
		ModuleId(s_ModuleId);
	}
	return s_ModuleId;
}

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
	s_InheritedMoney = SMoney();
	s_InheritedTrade.Reset();
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
			s_InheritedMoney		= { all[i].has_money, all[i].money_min, all[i].money_max, all[i].money_infinite };
			s_InheritedTrade		= all[i].trade;
			return;
		}
}

// The problems of the edited character as the build will see it: resolved against the
// others (its base, the ids they take), then validated. The same answer characters_write gives.
void Revalidate()
{
	s_OwnStale = false;
	s_OwnMounted = EditorGameContent::Mounted();
	s_Own.clear();
	if (s_Sel < 0 && !s_Dirty) return;
	xr_vector<EditorCharacters::SCharacter> all = s_All;
	u32 at = (s_Sel >= 0 && s_Sel < (int)all.size()) ? (u32)s_Sel : (u32)all.size();
	if (at == all.size())	all.push_back(s_Edit);
	else					all[at] = s_Edit;
	const xr_string where = s_Edit.file.empty() ? s_Edit.id : NqUtil::ProjectRelative(s_Edit.file.c_str());
	xr_vector<xr_string> chain;
	EditorCharacters::Resolve(all, chain);
	for (const xr_string& p : chain)
		if (p.find(" " + where + ": ") != xr_string::npos)
			s_Own.push_back(p);
	for (u32 i = 0; i < all.size(); ++i)
		if (i != at && all[i].id == s_Edit.id)
			s_Own.push_back("E101 " + where + ": another character has the id '" + s_Edit.id + "' - " +
				NqUtil::ProjectRelative(all[i].file.c_str()));
	EditorCharacters::Validate(all[at], s_OwnMounted, s_Own);
}

// the row a coded problem belongs under
LPCSTR FieldOf(const xr_string& problem)
{
	static const struct { LPCSTR code; LPCSTR field; } kMap[] =
	{
		{ "E101", "id" }, { "E102", "id" }, { "E111", "base" }, { "E112", "base" }, { "E103", "name" },
		{ "W107", "text" }, { "E104", "visual" }, { "W105", "visual" }, { "E123", "visual" }, { "W115", "icon" }, { "W114", "community" },
		{ "W116", "dialog" }, { "E110", "money" }, { "W117", "kit" }, { "E108", "kit" }, { "E109", "kit" },
		{ "E118", "trade" }, { "E119", "trade" }, { "W120", "trade" }, { "W121", "trade" }, { "W122", "trade" },
	};
	for (const auto& m : kMap)
		if (0 == problem.compare(0, 4, m.code))
			return m.field;
	return "";
}

// "E103 characters/x.npcasset: no name - ..." -> "No name - ...". The code and the file are for MCP and the log;
// the author reads the sentence under the field it is about.
xr_string ProblemText(const xr_string& problem)
{
	const size_t colon = problem.find(": ");
	xr_string text = colon == xr_string::npos ? problem.substr(_min(problem.size(), size_t(5))) : problem.substr(colon + 2);
	if (!text.empty() && text[0] >= 'a' && text[0] <= 'z')
		text[0] = char(text[0] - 'a' + 'A');
	if (!text.empty() && !strchr(".!?", text.back()))
		text += '.';
	return text;
}

bool FieldHasError(LPCSTR field)
{
	for (const xr_string& p : s_Own)
		if (p[0] == 'E' && !strcmp(FieldOf(p), field))
			return true;
	return false;
}

int RowFlags(LPCSTR field, int flags = 0)
{
	return flags | (FieldHasError(field) ? XUI::PropInvalid : 0);
}

void FieldMessages(LPCSTR field)
{
	for (const xr_string& p : s_Own)
		if (!strcmp(FieldOf(p), field))
			XUI::PropertyMessage(p[0] == 'E' ? XUI::Tone::Error : XUI::Tone::Warning, ProblemText(p).c_str());
}

// the same, as lines of their own outside the grid
void FieldStatus(LPCSTR field)
{
	for (const xr_string& p : s_Own)
		if (!strcmp(FieldOf(p), field))
			XUI::StatusText(p[0] == 'E' ? XUI::Tone::Error : XUI::Tone::Warning, ProblemText(p).c_str());
}

// A community a stalker can be of. The list of the linked game also carries the
// monsters' (their own section, plus "monster" - what every one of them is to a
// human) and the player's own, and none of those makes an NPC.
bool StalkerCommunity(const NqPickers::SEntry& e)
{
	if (e.extra == "monster_communities" || e.id == "monster")	return false;
	return e.id != "actor" && 0 != strncmp(e.id.c_str(), "actor_", 6);
}

// A combo's rows share one filter: one popup is open at a time.
char							s_Filter[96]	= "";
EditorCharacters::TModelPreview	s_ModelPreview	= 0;

// the filter on top of a picker popup, focused when it opens
void PickerFilter()
{
	if (ImGui::IsWindowAppearing())
	{
		s_Filter[0] = 0;
		ImGui::SetKeyboardFocusHere();
	}
	ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
	XUI::SearchField("##filter", s_Filter, sizeof(s_Filter), "Type to filter");
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
		const bool typed = ImGui::InputTextWithHint("##community", "stalker", s_Community, sizeof(s_Community));
		XUI::Tip("No linked game to list the communities of: type the id.");
		return typed;
	}

	const NqPickers::SEntry* cur = 0;
	for (u32 i = 0; i < all.size(); ++i)
		if (all[i].id == s_Community)	{ cur = &all[i]; break; }

	// unset is a value of its own: a character with a base takes the base's
	const xr_string unset = s_Edit.base.empty() ? xr_string("Not set: stalker") :
		"From " + s_Edit.base + (s_InheritedCommunity.empty() ? xr_string() : ": " + s_InheritedCommunity);
	xr_string preview;
	if (!s_Community[0])				preview = unset;
	else if (cur && !cur->name.empty())	preview = cur->name + "  (" + cur->id + ")";
	else								preview = s_Community;

	const bool unknown = s_Community[0] && !cur;
	if (unknown)	ImGui::PushStyleColor(ImGuiCol_Text, XUI::ToneColor(XUI::Tone::Warning));
	const bool open = ImGui::BeginCombo("##community", preview.c_str(), ImGuiComboFlags_HeightLarge);
	if (unknown)	ImGui::PopStyleColor();
	if (unknown)	XUI::Tip("Not a community of the linked game: kept as written, for one the module brings itself.");
	if (!open)	return false;
	bool picked = false;
	PickerFilter();
	if (XUI::Match(s_Filter, unset.c_str()) && ImGui::Selectable(unset.c_str(), !s_Community[0]))	{ s_Community[0] = 0; picked = true; }
	for (u32 i = 0; i < all.size(); ++i)
	{
		if (!StalkerCommunity(all[i]) && &all[i] != cur)	continue;
		const xr_string label = all[i].name.empty() ? all[i].id : all[i].name + "  (" + all[i].id + ")";
		if (!XUI::Match(s_Filter, label.c_str()))	continue;
		ImGui::PushID((int)i);
		if (ImGui::Selectable(label.c_str(), &all[i] == cur))
		{
			CopyTo(s_Community, sizeof(s_Community), all[i].id);
			picked = true;
		}
		ImGui::PopID();
	}
	ImGui::EndCombo();
	return picked;
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
// shows "caption (id)"; a value the list does not hold stays and is marked, never
// replaced - the text path may name what only the module brings.
bool ListPicker(LPCSTR label, char* buf, u32 size, const xr_vector<NqPickers::SEntry>& list, LPCSTR none)
{
	const NqPickers::SEntry* cur = FindEntry(list, buf);
	xr_string preview = buf[0] ? xr_string(buf) : xr_string(none ? none : "");
	if (cur && !cur->name.empty())	preview = cur->name + "  (" + buf + ")";
	const bool unknown = buf[0] && !cur && !list.empty();
	bool picked = false;
	if (unknown)	ImGui::PushStyleColor(ImGuiCol_Text, XUI::ToneColor(XUI::Tone::Warning));
	const bool open = ImGui::BeginCombo(label, preview.c_str(), ImGuiComboFlags_HeightLarge);
	if (unknown)	ImGui::PopStyleColor();
	if (unknown && !open)
		XUI::Tip((xr_string("'") + buf + "' is not in the linked game or the project: kept as written").c_str());
	if (!open)	return false;

	PickerFilter();
	if (none && ImGui::Selectable(none, !buf[0]))	{ buf[0] = 0; picked = true; }
	static xr_vector<u32> shown;
	shown.clear();
	for (u32 i = 0; i < list.size(); ++i)
		if (XUI::Match(s_Filter, (list[i].id + " " + list[i].name).c_str()))	shown.push_back(i);
	ImGuiListClipper clip;
	clip.Begin((int)shown.size());
	while (clip.Step())
		for (int r = clip.DisplayStart; r < clip.DisplayEnd; ++r)
		{
			const NqPickers::SEntry& e = list[shown[r]];
			ImGui::PushID(r);
			const xr_string row = e.name.empty() ? e.id : e.name + "  (" + e.id + ")";
			if (ImGui::Selectable(row.c_str(), e.id == buf))	{ CopyTo(buf, size, e.id); picked = true; }
			ImGui::PopID();
		}
	if (shown.empty())	XUI::StatusText(XUI::Tone::Muted, "Nothing matches.");
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

// The face the player sees, `height` px tall, or a frame with a person in it when there is none.
void DrawPortrait(LPCSTR icon, float height)
{
	const ImVec2 face(height * 1.42f, height);
	const NqPickers::SEntry* cur = FindEntry(NqPickers::Index(NqPickers::tIcon), icon);
	ImTextureID tex = 0;
	ImVec2 uv0, uv1;
	const ImVec2 at = ImGui::GetCursorScreenPos();
	ImDrawList* dl = ImGui::GetWindowDrawList();
	dl->AddRectFilled(at, ImVec2(at.x + face.x, at.y + face.y), GrizzlyTheme::U32(Token::Inset), XUI::Pt(4.f));
	if (cur && Portrait(*cur, tex, uv0, uv1))
		ImGui::Image(tex, face, uv0, uv1);
	else
	{
		dl->AddRect(at, ImVec2(at.x + face.x, at.y + face.y), GrizzlyTheme::U32(Token::BorderSubtle), XUI::Pt(4.f));
		const float glyph = _min(height, XUI::Pt(40.f)) * .6f;
		XUI::DrawIcon(dl, XUI::Icon::Person, ImVec2(at.x + (face.x - glyph) * .5f, at.y + (face.y - glyph) * .5f), glyph / XUI::Dpi(),
			GrizzlyTheme::U32(Token::TextMuted));
		ImGui::Dummy(face);
	}
	XUI::Tip(icon && icon[0] ? icon : "No portrait");
}

// The portrait, picked from a gallery of every face the linked game and the project have: by
// looking, not by remembering ui_inGame2_neutral_2_mask.
bool IconPicker(char* buf, u32 size)
{
	const xr_vector<NqPickers::SEntry>& list = NqPickers::Index(NqPickers::tIcon);
	const NqPickers::SEntry* cur = FindEntry(list, buf);
	const bool unknown = buf[0] && !cur && !list.empty();
	if (unknown)	ImGui::PushStyleColor(ImGuiCol_Text, XUI::ToneColor(XUI::Tone::Warning));
	const bool open = ImGui::BeginCombo("##icon", buf[0] ? buf : "None", ImGuiComboFlags_HeightLargest);
	if (unknown)	ImGui::PopStyleColor();
	if (!open)	return false;

	bool picked = false;
	PickerFilter();
	if (ImGui::Selectable("None", !buf[0]))	{ buf[0] = 0; picked = true; }
	static xr_vector<u32> shown;
	shown.clear();
	for (u32 i = 0; i < list.size(); ++i)
		if (XUI::Match(s_Filter, (list[i].id + " " + list[i].name).c_str()))	shown.push_back(i);

	// a gallery: as many columns as fit, only the rows in view drawn
	const ImGuiStyle& st = ImGui::GetStyle();
	const ImVec2 cell(XUI::Em(6.2f), XUI::Em(4.4f));
	const int cols = _max(1, int((ImGui::GetContentRegionAvail().x + st.ItemSpacing.x) /
								(cell.x + st.FramePadding.x * 2.f + st.ItemSpacing.x)));
	ImGuiListClipper clip;
	clip.Begin((int(shown.size()) + cols - 1) / cols, cell.y + st.FramePadding.y * 2.f + st.ItemSpacing.y);
	ImTextureID tex = 0;
	ImVec2 uv0, uv1;
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
				XUI::Tip((e.id + (0 == e.extra.compare(0, 8, "project:") ? xr_string("\nThe project's") : xr_string())).c_str());
				if (e.id == buf)
					ImGui::GetWindowDrawList()->AddRect(ImGui::GetItemRectMin(), ImGui::GetItemRectMax(),
						GrizzlyTheme::U32(Token::AccentBright), XUI::Pt(3.f), 0, XUI::Pt(2.f));
				ImGui::PopID();
			}
	if (shown.empty())	XUI::StatusText(XUI::Tone::Muted, "Nothing matches.");
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
	auto name = [](const NqPickers::SEntry* e) { return e->name.empty() ? e->id : e->name + "  (" + e->id + ")"; };
	const xr_string preview = !has_rank ? xr_string("Not set") : cur < 0 ? xr_string("Below the lowest rank") : name(ranks[cur]);
	bool picked = false;
	if (ImGui::BeginCombo("##rankname", preview.c_str()))
	{
		for (u32 i = 0; i < ranks.size(); ++i)
		{
			const int from = atoi(ranks[i]->extra.c_str());
			const int to = (i + 1 < ranks.size()) ? atoi(ranks[i + 1]->extra.c_str()) - 1 : -1;
			const xr_string item = name(ranks[i]) + (to >= 0 ? NqUtil::Format("   %d-%d", from, to) : NqUtil::Format("   %d+", from));
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
	if (!known)	ImGui::PushStyleColor(ImGuiCol_Text, XUI::ToneColor(XUI::Tone::Warning));
	const bool open = ImGui::BeginCombo("##base", s_Base[0] ? s_Base : "None", ImGuiComboFlags_HeightLarge);
	if (!known)	ImGui::PopStyleColor();
	if (!open)	return false;
	if (ImGui::Selectable("None", !s_Base[0]))	{ s_Base[0] = 0; picked = true; }
	for (u32 i = 0; i < s_All.size(); ++i)
	{
		// not itself: a character that starts from itself is E112
		if ((int)i == s_Sel)	continue;
		ImGui::PushID((int)i);
		const xr_string label = s_All[i].abstract_ ? s_All[i].id + "  (template)" : s_All[i].id;
		if (ImGui::Selectable(label.c_str(), s_All[i].id == s_Base))
		{
			CopyTo(s_Base, sizeof(s_Base), s_All[i].id);
			picked = true;
		}
		ImGui::PopID();
	}
	ImGui::EndCombo();
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
	s_OwnStale = true;
	PullToBuffers();
	RecomputeInherited();
}

// The list again from disk. Edits in progress stay: the edited character is found again by its file.
void ReloadAll()
{
	const xr_string file = s_Edit.file;
	s_Problems.clear();
	EditorCharacters::LoadAll(EditorProject::Root(), s_All, s_Problems);
	s_NeedReload = false;
	s_OwnStale = true;
	int found = -1;
	for (u32 i = 0; i < s_All.size(); ++i)
		if (!file.empty() && 0 == _stricmp(s_All[i].file.c_str(), file.c_str()))	found = (int)i;
	if (s_Dirty)
	{
		s_Sel = found;
		return;
	}
	SelectIndex(found >= 0 ? found : (s_All.empty() ? -1 : 0));
}

bool SaveEdited(xr_string& err)
{
	if (s_Edit.id.empty())	{ err = "the character needs an id"; return false; }
	// saved as it is edited, a new character is on disk under its first id: the file follows the id the author settles on
	if (!s_Edit.file.empty())
	{
		const xr_string folder = xr_string(EditorProject::Root()) + "\\characters\\", wanted = folder + s_Edit.id + ".npcasset";
		if (0 == _strnicmp(s_Edit.file.c_str(), folder.c_str(), folder.size()) && 0 != _stricmp(s_Edit.file.c_str(), wanted.c_str()) && !NqUtil::FileExists(wanted.c_str())
			&& ::MoveFileA(s_Edit.file.c_str(), wanted.c_str()))
			s_Edit.file = wanted;
	}
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
	s_Status.clear();
	return true;
}

void Discard()
{
	SelectIndex(s_Sel);
	s_Status.clear();
}

// Unsaved edits ask before anything replaces them: Save, Don't Save, or stay.
void AskThen(std::function<void()> action)
{
	if (!s_Dirty)
	{
		action();
		return;
	}
	XUI::Strings what;
	what.push_back(s_Edit.id.empty() ? xr_string("the new character") : s_Edit.id);
	XUI::UnsavedPrompt("character", what, [action](XUI::Answer answer)
	{
		if (answer == XUI::Answer::Cancel) return;
		if (answer == XUI::Answer::Yes)
		{
			xr_string err;
			if (!SaveEdited(err))
			{
				s_Status = "Not saved: " + err;
				return;
			}
			ReloadAll();
		}
		else
			Discard();
		action();
	});
}

void NewCharacter()
{
	EditorCharacters::SCharacter c;
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
	s_OwnStale	= true;
	PullToBuffers();
	RecomputeInherited();
}

// Names what disappears: the file goes for good, the module loses the character and a quest
// that spawns it has nothing to spawn.
void AskDelete()
{
	if (s_Edit.file.empty()) return;
	const xr_string file = s_Edit.file;
	const xr_string headline = "Delete the character \"" + s_Edit.id + "\"?";
	XUI::ConfirmSpec spec;
	spec.id = "delete_character";
	spec.title = "Delete Character";
	spec.headline = headline.c_str();
	spec.items.push_back(NqUtil::ProjectRelative(file.c_str()));
	spec.consequence = "Deleted for good - this cannot be undone. The next build takes the character out of the module, "
		"and a quest that spawns it has nothing to spawn.";
	spec.action = "Delete";
	spec.destructive = true;
	XUI::Confirm(spec, [file](XUI::Answer answer)
	{
		if (answer != XUI::Answer::Yes) return;
		EditorFileOps::SReport rep;
		EditorFileOps::Delete(file.c_str(), false, rep);
		EditorCharacters::Invalidate();
		s_Status = rep.ok() ? xr_string() : "Not deleted: " + (rep.failures.empty() ? xr_string("the file is in use") : rep.failures[0].reason);
		if (rep.ok())
		{
			s_Dirty = false;
			s_Edit.Reset();
			s_Sel = -1;
		}
		s_NeedReload = true;
	});
}

void XUI_LocalDirtyDot(const ImVec2& centre)
{
	ImGui::GetWindowDrawList()->AddCircleFilled(centre, XUI::Pt(3.5f), GrizzlyTheme::U32(Token::Text));
}

// ---- the list -----------------------------------------------------------------------
void DrawList()
{
	const float plus = XUI::Pt(30.f);
	ImGui::SetNextItemWidth(_max(XUI::Em(4.f), ImGui::GetContentRegionAvail().x - plus));
	XUI::SearchField("##char_search", s_ListFilter, sizeof(s_ListFilter), "Search characters");
	ImGui::SameLine(0.f, XUI::Pt(4.f));
	if (XUI::IconButton("##new_character", XUI::Icon::Plus, "Add a character", false, EditorProject::Active()))
		AskThen([] { NewCharacter(); });
	const float problems = s_Problems.empty() ? 0.f : ImGui::GetTextLineHeightWithSpacing() / XUI::Dpi() + 4.f;
	if (XUI::BeginList("##char_list", ImVec2(0.f, -problems)))
	{
		// a new character nobody saved yet heads the list
		if (s_Sel < 0 && s_Dirty)
		{
			XUI::ListRow("##new", true);
			const ImVec2 row_max = ImGui::GetItemRectMax();
			XUI::TextEllipsis(s_Edit.id.empty() ? "New character" : s_Edit.id.c_str(), _max(0.f, row_max.x - ImGui::GetCursorScreenPos().x - XUI::Pt(18.f)));
			XUI::Tip("Not saved yet");
			XUI_LocalDirtyDot(ImVec2(row_max.x - XUI::Pt(10.f), (ImGui::GetItemRectMin().y + ImGui::GetItemRectMax().y) * .5f));
		}
		int shown = 0;
		for (u32 i = 0; i < s_All.size(); ++i)
		{
			const EditorCharacters::SCharacter& c = s_All[i];
			const xr_string name = TextIn(c.name, "rus");
			if (!XUI::Match(s_ListFilter, (c.id + " " + name + " " + TextIn(c.name, "eng")).c_str()))	continue;
			++shown;
			const bool selected = s_Sel == (int)i;
			ImGui::PushID((int)i);
			const bool clicked = XUI::ListRow(c.id.c_str(), selected);
			const ImVec2 row_min = ImGui::GetItemRectMin(), row_max = ImGui::GetItemRectMax();
			const float cy = (row_min.y + row_max.y) * .5f;
			const float x = ImGui::GetCursorScreenPos().x;
			XUI::DrawIcon(ImGui::GetWindowDrawList(), c.abstract_ ? XUI::Icon::CharacterTemplate : XUI::Icon::Character,
				ImVec2(floorf(x + .5f), floorf(cy - XUI::Pt(7.f) + .5f)), 14.f, GrizzlyTheme::U32(selected ? Token::AccentBright : Token::TextSecondary));
			ImGui::SetCursorPosX(ImGui::GetCursorPosX() + XUI::Pt(20.f));
			// what sits at the right end: the template mark, the dot of unsaved edits
			const float right = (c.abstract_ ? XUI::Em(4.5f) : 0.f) + (selected && s_Dirty ? XUI::Pt(16.f) : 0.f) + XUI::Pt(6.f);
			const float room = _max(0.f, row_max.x - ImGui::GetCursorScreenPos().x - right);
			const float id_w = _min(ImGui::CalcTextSize(c.id.c_str()).x, room);
			XUI::TextEllipsis(c.id.c_str(), id_w);
			XUI::Tip((NqUtil::ProjectRelative(c.file.c_str()) + (name.empty() ? xr_string() : "\n" + name)).c_str());
			if (!name.empty() && room - id_w > XUI::Em(3.f))
			{
				ImGui::SameLine(0.f, XUI::Pt(8.f));
				ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(Token::TextMuted));
				XUI::TextEllipsis(name.c_str(), room - id_w - XUI::Pt(8.f));
				ImGui::PopStyleColor();
			}
			if (c.abstract_)
			{
				ImGui::SameLine();
				ImGui::SetCursorScreenPos(ImVec2(row_max.x - right + XUI::Pt(2.f), ImGui::GetCursorScreenPos().y));
				XUI::Badge("Template");
			}
			if (selected && s_Dirty)
				XUI_LocalDirtyDot(ImVec2(row_max.x - XUI::Pt(10.f), cy));
			ImGui::PopID();
			if (clicked && !selected)
			{
				const int index = (int)i;
				AskThen([index] { SelectIndex(index); });
			}
		}
		if (s_All.empty() && !(s_Sel < 0 && s_Dirty))
		{
			if (XUI::EmptyState(XUI::Icon::Character, "No characters", "The module's own NPCs live here.", "New Character"))
				NewCharacter();
		}
		else if (!shown && s_ListFilter[0])
			XUI::StatusText(XUI::Tone::Muted, "No character matches.");
	}
	XUI::EndList();
	if (!s_Problems.empty())
	{
		XUI::StatusText(XUI::Tone::Error, s_Problems.size() == 1 ? "1 file does not read" : NqUtil::Format("%u files do not read", (u32)s_Problems.size()).c_str());
		xr_string all;
		for (const xr_string& p : s_Problems) all += (all.empty() ? "" : "\n") + p;
		XUI::Tip(all.c_str());
	}
}

// ---- the form -----------------------------------------------------------------------
// true when something changed
bool DrawKit()
{
	bool changed = false;
	const xr_vector<NqPickers::SEntry>& items = NqPickers::Index(NqPickers::tItem);
	const ImGuiTableFlags flags = ImGuiTableFlags_RowBg | ImGuiTableFlags_BordersInnerH | ImGuiTableFlags_PadOuterX;
	if (s_Inherited.empty() && s_Edit.supplies.empty())
		XUI::StatusText(XUI::Tone::Muted, "No starting kit: the NPC spawns with what its community gives.");
	else if (ImGui::BeginTable("##kit", 3, flags))
	{
		ImGui::TableSetupColumn("Item", ImGuiTableColumnFlags_WidthStretch);
		ImGui::TableSetupColumn("Count", ImGuiTableColumnFlags_WidthFixed, XUI::Em(5.f));
		ImGui::TableSetupColumn("##actions", ImGuiTableColumnFlags_WidthFixed, XUI::Pt(24.f));
		XUI::TableHeaders();
		// what the base already gives: shown, not editable, and not repeated below
		for (u32 i = 0; i < s_Inherited.size(); ++i)
		{
			const NqPickers::SEntry* it = FindEntry(items, s_Inherited[i].section.c_str());
			ImGui::TableNextRow(0, XUI::Pt(26.f));
			ImGui::TableNextColumn();
			ImGui::AlignTextToFramePadding();
			ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(Token::TextMuted));
			XUI::TextEllipsis(((it && !it->name.empty() ? it->name + "  (" + s_Inherited[i].section + ")" : s_Inherited[i].section) +
				"  - from " + s_Edit.base).c_str());
			XUI::Tip(("From " + s_Edit.base + ". A row of the same item below changes the count; a count of 0 takes it out.").c_str());
			ImGui::TableNextColumn();
			ImGui::AlignTextToFramePadding();
			XUI::TextEllipsis(NqUtil::Format("%d", s_Inherited[i].count).c_str());
			ImGui::PopStyleColor();
			ImGui::TableNextColumn();
		}
		int remove = -1;
		for (u32 i = 0; i < s_Edit.supplies.size(); ++i)
		{
			ImGui::PushID((int)i);
			ImGui::TableNextRow(0, XUI::Pt(26.f));
			ImGui::TableNextColumn();
			char section[128];
			CopyTo(section, sizeof(section), s_Edit.supplies[i].section);
			ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
			if (ListPicker("##sect", section, sizeof(section), items, 0))
			{
				s_Edit.supplies[i].section = section;
				changed = true;
			}
			ImGui::TableNextColumn();
			ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
			if (ImGui::InputInt("##count", &s_Edit.supplies[i].count, 0, 0))	changed = true;
			// 0 stays: with a base it means "not this one". Below zero never meant anything.
			if (s_Edit.supplies[i].count < 0)	s_Edit.supplies[i].count = 0;
			if (s_Edit.supplies[i].count == 0 && s_Inherited.empty())
				s_Edit.supplies[i].count = 1;
			ImGui::TableNextColumn();
			if (XUI::IconButton("##remove", XUI::Icon::Delete, "Take the row out"))	remove = (int)i;
			ImGui::PopID();
		}
		ImGui::EndTable();
		if (remove >= 0)
		{
			s_Edit.supplies.erase(s_Edit.supplies.begin() + remove);
			changed = true;
		}
	}
	FieldStatus("kit");
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
	if (XUI::Button("Add Item"))
	{
		s_Edit.supplies.push_back(EditorCharacters::SSupply());
		changed = true;
	}
	return changed;
}

// ---- trade ----------------------------------------------------------------------------
// A multi-pick over the names a list offers: the preview joins what is picked, the popup ticks
// each one. A name the list does not hold stays picked, says so, and can be unticked.
bool XUI_LocalMultiCombo(LPCSTR id, xr_vector<xr_string>& picked, const xr_vector<xr_string>& names, LPCSTR none)
{
	xr_string preview;
	for (u32 i = 0; i < picked.size(); ++i) { if (i) preview += ", "; preview += picked[i]; }
	bool changed = false;
	if (ImGui::BeginCombo(id, preview.empty() ? none : preview.c_str()))
	{
		auto toggle = [&](const xr_string& name, LPCSTR caption)
		{
			bool on = false;
			for (u32 k = 0; k < picked.size(); ++k) if (picked[k] == name) on = true;
			if (ImGui::Checkbox(caption, &on))
			{
				if (on) picked.push_back(name);
				else for (u32 k = 0; k < picked.size(); ++k) if (picked[k] == name) { picked.erase(picked.begin() + k); break; }
				changed = true;
			}
		};
		for (u32 i = 0; i < names.size(); ++i) toggle(names[i], names[i].c_str());
		const xr_vector<xr_string> copy = picked;
		for (u32 i = 0; i < copy.size(); ++i)
		{
			bool listed = false;
			for (u32 k = 0; k < names.size(); ++k) if (names[k] == copy[i]) listed = true;
			if (!listed) toggle(copy[i], (copy[i] + "  (not in the linked game)").c_str());
		}
		if (names.empty())
			XUI::StatusText(XUI::Tone::Muted, "The linked game lists no trade categories.");
		ImGui::EndCombo();
	}
	return changed;
}

xr_string TradeSummary(const EditorCharacters::STrade& t)
{
	xr_string sells;
	for (u32 i = 0; i < t.sells.size(); ++i) { if (i) sells += ", "; sells += t.sells[i]; }
	return NqUtil::Format("sells %s, a stock of %u item(s), asks %g and pays %g of the cost",
		sells.empty() ? "its stock only" : sells.c_str(), (u32)t.stock.size(), t.markup, t.pays);
}

// Not a trader, or trading as the base does: a line saying which, and the way to change it.
bool DrawTradeOff()
{
	bool changed = false;
	// the kit above has buttons of the same names; each part pushes and pops inside itself,
	// never across the property grid, which is a table with an id of its own
	ImGui::PushID("trade");
	const bool from_base = !s_Edit.trade.declared && s_InheritedTrade.Trader();
	if (from_base)
		XUI::Prose(("Trades as " + s_Edit.base + " does: " + TradeSummary(s_InheritedTrade) + ".").c_str());
	else if (s_Edit.trade.declared && s_Edit.trade.off && s_InheritedTrade.Trader())
		XUI::Prose(("Not a trader, although " + s_Edit.base + " is one.").c_str());
	else
		XUI::Prose("Not a trader: it trades like any stalker of its community, with what it carries.");
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
	if (XUI::Button(from_base ? "Change Trade" : "Make a Trader", XUI::Kind::Secondary))
	{
		if (s_InheritedTrade.Trader())	s_Edit.trade = s_InheritedTrade;
		else							s_Edit.trade.Reset();
		s_Edit.trade.declared	= true;
		s_Edit.trade.off		= false;
		changed = true;
	}
	XUI::Tip("A shop of its own: what it sells and buys, its stock and its prices. The game uses it whatever the NPC is doing.");
	if (from_base)
	{
		ImGui::SameLine();
		if (XUI::Button("Not a Trader"))
		{
			s_Edit.trade.Reset();
			s_Edit.trade.declared	= true;
			s_Edit.trade.off		= true;
			changed = true;
		}
		XUI::Tip("Takes the base's shop away from this character.");
	}
	if (s_Edit.trade.declared && s_Edit.trade.off)
	{
		ImGui::SameLine();
		if (XUI::Button("Clear"))
		{
			s_Edit.trade.Reset();
			changed = true;
		}
		XUI::Tip(s_Edit.base.empty() ? "Says nothing about trade" : "Trades as the base does");
	}
	ImGui::PopID();
	return changed;
}

// The shop's rows: what it sells and buys, and its prices.
bool DrawTradeRows()
{
	bool changed = false;
	ImGui::PushID("trade");
	EditorCharacters::STrade& t = s_Edit.trade;
	const xr_vector<xr_string>& cats = EditorCharacters::TradeCategories();
	if (XUI::PropertyRow("Sells", "Besides its stock, which it always sells: the categories of the linked game's trade presets "
		"it sells when it has them. None: only the stock.", RowFlags("trade")))
		changed |= XUI_LocalMultiCombo("##sells", t.sells, cats, "Only its stock");
	if (XUI::PropertyRow("Buys", "The categories it buys from the player. None: it buys nothing."))
		changed |= XUI_LocalMultiCombo("##buys", t.buys, cats, "Nothing");
	if (XUI::PropertyRow("Markup", "What it asks, as a share of an item's cost: 1 is the cost, 1.5 half as much again."))
		if (ImGui::InputFloat("##markup", &t.markup, 0.f, 0.f, "%.2f"))	changed = true;
	if (XUI::PropertyRow("Pays", "What it pays for an item, as a share of its cost - over the factors the game gives single "
		"items, the way the game's own traders do: 0.3 is about a third."))
		if (ImGui::InputFloat("##pays", &t.pays, 0.f, 0.f, "%.2f"))	changed = true;
	if (XUI::PropertyRow("Restock", "Game hours until the stock comes back."))
		if (ImGui::InputInt("##restock", &t.restock, 0, 0))	changed = true;
	ImGui::PopID();
	return changed;
}

// The stock: what is on its shelf after every restock, each line with its chance.
bool DrawStock()
{
	bool changed = false;
	ImGui::PushID("trade");
	EditorCharacters::STrade& t = s_Edit.trade;
	const xr_vector<NqPickers::SEntry>& items = NqPickers::Index(NqPickers::tItem);
	const ImGuiTableFlags flags = ImGuiTableFlags_RowBg | ImGuiTableFlags_BordersInnerH | ImGuiTableFlags_PadOuterX;
	XUI::BlockLabel("Stock", "What is on the shelf after every restock: an item, how many, and the chance that it is there.");
	if (t.stock.empty())
		XUI::StatusText(XUI::Tone::Muted, "Nothing on the shelf: it sells only what its categories let it sell of what it carries.");
	else if (ImGui::BeginTable("##stock", 4, flags))
	{
		ImGui::TableSetupColumn("Item", ImGuiTableColumnFlags_WidthStretch);
		ImGui::TableSetupColumn("Count", ImGuiTableColumnFlags_WidthFixed, XUI::Em(4.5f));
		ImGui::TableSetupColumn("Chance", ImGuiTableColumnFlags_WidthFixed, XUI::Em(4.5f));
		ImGui::TableSetupColumn("##actions", ImGuiTableColumnFlags_WidthFixed, XUI::Pt(24.f));
		XUI::TableHeaders();
		int remove = -1;
		for (u32 i = 0; i < t.stock.size(); ++i)
		{
			ImGui::PushID((int)i);
			ImGui::TableNextRow(0, XUI::Pt(26.f));
			ImGui::TableNextColumn();
			char section[128];
			CopyTo(section, sizeof(section), t.stock[i].section);
			ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
			if (ListPicker("##sect", section, sizeof(section), items, 0))
			{
				t.stock[i].section = section;
				changed = true;
			}
			ImGui::TableNextColumn();
			ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
			if (ImGui::InputInt("##count", &t.stock[i].count, 0, 0))	changed = true;
			if (t.stock[i].count < 1)	t.stock[i].count = 1;
			ImGui::TableNextColumn();
			ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
			if (ImGui::InputFloat("##chance", &t.stock[i].chance, 0.f, 0.f, "%.2f"))	changed = true;
			ImGui::TableNextColumn();
			if (XUI::IconButton("##remove", XUI::Icon::Delete, "Take the row out"))	remove = (int)i;
			ImGui::PopID();
		}
		ImGui::EndTable();
		if (remove >= 0)
		{
			t.stock.erase(t.stock.begin() + remove);
			changed = true;
		}
	}
	FieldStatus("trade");
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
	if (XUI::Button("Add Item"))
	{
		t.stock.push_back(EditorCharacters::STradeRow());
		changed = true;
	}
	ImGui::SameLine();
	if (XUI::Button("Not a Trader"))
	{
		const bool base_trades = s_InheritedTrade.Trader();
		t.Reset();
		// with a base that trades, saying nothing would bring its shop back
		t.declared = base_trades;
		t.off = base_trades;
		changed = true;
	}
	XUI::Tip("The NPC keeps no shop: it trades like any stalker of its community.");
	ImGui::PopID();
	return changed;
}

void DrawHeader(float width)
{
	const xr_string& mod = CachedModuleId();
	const float portrait = XUI::Pt(64.f);
	ImGui::BeginGroup();
	DrawPortrait(s_Icon, portrait);
	ImGui::SameLine(0.f, XUI::Pt(12.f));
	ImGui::BeginGroup();
	const float delete_w = ImGui::CalcTextSize("Delete...").x + XUI::Pt(20.f);
	const float text_w = _max(XUI::Em(4.f), width - portrait * 1.42f - XUI::Pt(12.f) - delete_w - XUI::Pt(8.f));
	xr_string title = s_NameEng[0] ? s_NameEng : s_NameRus;
	if (title.empty()) title = s_Edit.id.empty() ? xr_string("New character") : s_Edit.id;
	{
		XUI::FontScope font(XUI::Font::Title);
		XUI::TextEllipsis(title.c_str(), text_w);
	}
	ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(Token::TextSecondary));
	XUI::TextEllipsis(mod.empty() ? "The module has no id yet: Mod Settings" : ("In game: " + EditorCharacters::GameId(mod.c_str(), s_Edit.id.c_str())).c_str(), text_w);
	XUI::Tip("The spawn section, the squad section (with _squad) and the profile a placed stalker carries.");
	XUI::TextEllipsis(s_Edit.file.empty() ? "Not saved yet" : NqUtil::ProjectRelative(s_Edit.file.c_str()).c_str(), text_w, XUI::Clip::Start);
	ImGui::PopStyleColor();
	if (s_Edit.abstract_)
		XUI::Badge("Template");
	ImGui::EndGroup();
	ImGui::EndGroup();
	ImGui::SameLine(_max(0.f, width - delete_w));
	if (XUI::Button("Delete...", XUI::Kind::GhostDanger, !s_Edit.file.empty(), "Not saved yet: nothing to delete"))
		AskDelete();
	for (const xr_string& p : s_Own)
		if (!FieldOf(p)[0])
			XUI::StatusText(p[0] == 'E' ? XUI::Tone::Error : XUI::Tone::Warning, ProblemText(p).c_str());
	if (!s_Status.empty())
		XUI::StatusText(XUI::Tone::Error, s_Status.c_str());
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(6.f)));
}

// One amount of the Money row: empty while the character sets no money of its own, the base's amount (or 0) as
// the hint. A number typed in sets the money, starting from what the row showed.
bool MoneyField(LPCSTR id, int& value, int hint)
{
	char text[16] = "";
	if (s_Edit.has_money)
		sprintf_s(text, "%d", value);
	char hint_text[16];
	sprintf_s(hint_text, "%d", hint);
	if (!ImGui::InputTextWithHint(id, hint_text, text, sizeof(text), ImGuiInputTextFlags_CharsDecimal))
		return false;
	if (!s_Edit.has_money)
	{
		s_Edit.has_money		= true;
		s_Edit.money_min		= s_InheritedMoney.min;
		s_Edit.money_max		= s_InheritedMoney.max;
		s_Edit.money_infinite	= s_InheritedMoney.infinite;
	}
	value = _max(0, atoi(text));
	return true;
}

// Money: from, to and Never runs out, all on the row; Clear goes back to the base's money, or to none
bool MoneyRow()
{
	bool changed = false;
	const float full = ImGui::CalcItemWidth();
	const float x0 = ImGui::GetCursorPosX();
	const float number = XUI::Em(5.5f);
	const float spacing = ImGui::GetStyle().ItemSpacing.x;
	ImGui::SetNextItemWidth(number);
	changed |= MoneyField("##money_min", s_Edit.money_min, s_InheritedMoney.min);
	ImGui::SameLine();
	ImGui::AlignTextToFramePadding();
	ImGui::TextUnformatted("to");
	ImGui::SameLine();
	ImGui::SetNextItemWidth(number);
	changed |= MoneyField("##money_max", s_Edit.money_max, s_InheritedMoney.max);
	// the checkbox goes under the numbers when the row is too narrow for it
	const float used = number * 2.f + ImGui::CalcTextSize("to").x + spacing * 3.f;
	const float box = ImGui::GetFrameHeight() + ImGui::GetStyle().ItemInnerSpacing.x + ImGui::CalcTextSize("Never runs out").x;
	if (used + box <= full)
		ImGui::SameLine();
	else
		ImGui::SetCursorPosX(x0);
	bool infinite = s_Edit.has_money ? s_Edit.money_infinite : s_InheritedMoney.infinite;
	if (ImGui::Checkbox("Never runs out", &infinite))
	{
		if (!s_Edit.has_money)
		{
			s_Edit.has_money	= true;
			s_Edit.money_min	= s_InheritedMoney.min;
			s_Edit.money_max	= s_InheritedMoney.max;
		}
		s_Edit.money_infinite = infinite;
		changed = true;
	}
	XUI::Tip("Trading never lowers it.");
	if (s_Edit.has_money && XUI::PropertyTrailing("##money_clear", XUI::Icon::Clear,
		s_Edit.base.empty() ? "Clear: the NPC spawns without money" : "Clear: the base's money applies"))
	{
		s_Edit.has_money = false;
		s_Edit.money_min = s_Edit.money_max = 0;
		s_Edit.money_infinite = false;
		changed = true;
	}
	return changed;
}

bool DrawForm(float width)
{
	bool changed = false;
	const xr_string& mod = CachedModuleId();
	DrawHeader(width);

	if (XUI::BeginPropertyGrid("character", .30f))
	{
		if (XUI::PropertySection("Identity"))
		{
			if (XUI::PropertyRow("ID", NqUtil::Format("a-z, 0-9 and _. The game knows the character as %s_<id>.",
				mod.empty() ? "<module>" : mod.c_str()).c_str(), RowFlags("id", XUI::PropRequired)))
			{
				if (ImGui::InputText("##id", s_Id, sizeof(s_Id)))	changed = true;
				FieldMessages("id");
			}
			if (XUI::PropertyRow("Base", "Another character to start from: what this one does not set comes from there, and "
				"the starting kits are merged. None: the character stands on its own.", RowFlags("base")))
			{
				if (BaseCombo())
				{
					changed = true;
					s_Edit.base = s_Base;
					RecomputeInherited();
				}
				FieldMessages("base");
			}
			if (XUI::PropertyRow("Template", "A template only: never a character in the game, so it needs no name and no model. "
				"It holds what several characters share."))
				if (ImGui::Checkbox("##abstract", &s_Edit.abstract_))	changed = true;
			XUI::EndPropertySection();
		}
		// Both languages on screen at once: a dropdown that swaps one field cannot show what the
		// other holds, and the text a character carries is what an author compares side by side.
		if (XUI::PropertySection("Name and Bio"))
		{
			if (XUI::PropertyRow("Name (rus)", "What the player reads above the NPC and in dialog.", RowFlags("name")))
			{
				if (ImGui::InputText("##name_rus", s_NameRus, sizeof(s_NameRus)))	changed = true;
				FieldMessages("name");
			}
			if (XUI::PropertyRow("Name (eng)", "Left empty, an English install reads the Russian name."))
				if (ImGui::InputText("##name_eng", s_NameEng, sizeof(s_NameEng)))	changed = true;
			XUI::EndPropertyGrid();
			changed |= XUI::TextBlock("Bio (rus)", "The line the PDA shows about the character.", "##bio_rus", s_BioRus, sizeof(s_BioRus), 2);
			changed |= XUI::TextBlock("Bio (eng)", "Left empty, an English install reads the Russian bio.", "##bio_eng", s_BioEng, sizeof(s_BioEng), 2);
			FieldStatus("text");
			XUI::BeginPropertyGrid("character", .30f);
		}
		if (XUI::PropertySection("Appearance"))
		{
			if (XUI::PropertyRow("Model", "The model the NPC spawns with and keeps: meshes\\actors of the linked game and the project. "
				"An outfit changes its protection, never its model.", RowFlags("visual")))
			{
				if (ListPicker("##visual", s_Visual, sizeof(s_Visual), Visuals(), s_Edit.base.empty() ? "Not set" : "From the base"))
					changed = true;
				if (XUI::PropertyTrailing("##preview", XUI::Icon::Eye, s_ModelPreview && s_Visual[0] ? "Preview the model" : "Pick a model first") &&
					s_ModelPreview && s_Visual[0])
					s_ModelPreview(s_Visual);
				FieldMessages("visual");
			}
			if (XUI::PropertyRow("Portrait", "The face the dialog window and the PDA show. The gallery holds every portrait of the "
				"linked game and the project.", RowFlags("icon")))
			{
				if (IconPicker(s_Icon, sizeof(s_Icon)))	changed = true;
				FieldMessages("icon");
			}
			XUI::EndPropertySection();
		}
		if (XUI::PropertySection("Profile"))
		{
			if (XUI::PropertyRow("Community", "The communities of the linked game. Not set: the base's, or stalker without one.", RowFlags("community")))
			{
				if (CommunityCombo())	changed = true;
				FieldMessages("community");
			}
			if (XUI::PropertyRow("Rank", "The linked game's ranks by name; the number beside it is the exact value."))
			{
				const float number = XUI::Em(6.f);
				const float full = ImGui::CalcItemWidth();
				ImGui::SetNextItemWidth(_max(XUI::Em(6.f), full - number - ImGui::GetStyle().ItemSpacing.x));
				if (RankPicker(s_Edit.rank, s_Edit.has_rank))	changed = true;
				ImGui::SameLine();
				ImGui::SetNextItemWidth(number);
				if (ImGui::InputInt("##rank", &s_Edit.rank, 0, 0))	{ changed = true; s_Edit.has_rank = true; }
			}
			if (XUI::PropertyRow("Money", "What the NPC carries when it spawns: a random amount from the first number to the second. "
				"Empty: the base's money, or none.", RowFlags("money")))
			{
				if (MoneyRow())	changed = true;
				FieldMessages("money");
			}
			if (XUI::PropertyRow("Squad", "Also a one-man squad section, so a quest can create the character with spawn.squad "
				"as well as with spawn.npc."))
				if (ImGui::Checkbox("##squad", &s_Edit.squad))	changed = true;
			XUI::EndPropertySection();
		}
		if (XUI::PropertySection("Dialogs"))
		{
			if (XUI::PropertyRow("Opening Dialog", "The dialog it opens with (start_dialog), captioned with its first line.", RowFlags("dialog")))
			{
				if (ListPicker("##dialog", s_Dialog, sizeof(s_Dialog), NqPickers::Index(NqPickers::tDialog), "None"))
					changed = true;
				FieldMessages("dialog");
			}
			XUI::EndPropertySection();
		}
		if (XUI::PropertySection("Starting Kit"))
		{
			XUI::EndPropertyGrid();
			changed |= DrawKit();
			XUI::BeginPropertyGrid("character", .30f);
		}
		if (XUI::PropertySection("Trade", true, s_Edit.trade.Trader() ? "Trader" : nullptr))
		{
			if (s_Edit.trade.Trader())
			{
				changed |= DrawTradeRows();
				XUI::EndPropertyGrid();
				changed |= DrawStock();
			}
			else
			{
				XUI::EndPropertyGrid();
				changed |= DrawTradeOff();
			}
			XUI::BeginPropertyGrid("character", .30f);
		}
		XUI::EndPropertyGrid();
	}
	return changed;
}

void DrawToolbar(XUI::Toolbar& tb)
{
	const bool project = EditorProject::Active();
	tb.Group(1);
	if (tb.Action("reload", XUI::Icon::Refresh, "Reload", "Read the characters, the linked game's lists and the project's models again", project))
	{
		s_NeedReload = true;
		// the project's own dialogs, portraits and models may have changed on disk
		NqPickers::Invalidate();
		s_VisualsFor.clear();
		DropSheets();
	}
}
} // namespace

namespace EditorCharacters
{
static void DrawCharactersBody();
}

namespace
{
// Characters is one of the editor's major tabs (XUI::Shell), a tool without a file of its own
void ShowTab(bool activate)
{
	if (XUI::Shell::IsOpen("characters"))
	{
		if (activate) XUI::Shell::Activate("characters");
		return;
	}
	XUI::Shell::DocDesc d;
	d.id = "characters";
	d.kind = "characters";
	d.icon = XUI::Icon::Character;
	d.label = [] { return xr_string("Characters"); };
	d.tooltip = [] { return xr_string("The module's own NPCs"); };
	d.alive = [] { return s_Open; };
	d.dirty = [] { return s_Dirty; };
	d.toolbar = [](XUI::Toolbar& tb) { DrawToolbar(tb); };
	d.body = [] { EditorCharacters::DrawCharactersBody(); };
	d.save = [](xr_string& err) { return SaveEdited(err); };
	d.discard = [] { Discard(); };
	d.on_close = [] { s_Open = false; };
	XUI::Shell::Open(d, activate);
}

// the list as the disk has it, and a character asked for by the content browser selected
void Sync()
{
	if (!EditorProject::Active())
	{
		s_All.clear();
		s_Sel = -1;
		return;
	}
	if (s_NeedReload)
		ReloadAll();
	if (s_PendingOpen.empty())
		return;
	int index = -1;
	for (u32 i = 0; i < s_All.size(); ++i)
		if (0 == _stricmp(s_All[i].file.c_str(), s_PendingOpen.c_str()))	index = (int)i;
	s_PendingOpen.clear();
	if (index >= 0 && index != s_Sel)
		AskThen([index] { SelectIndex(index); });
}
}

bool EditorCharacters::IsOpen()	{ return s_Open; }

void EditorCharacters::Show()
{
	s_Open			= true;
	s_NeedReload	= true;
	ShowTab(true);
}

void EditorCharacters::Open(LPCSTR path)
{
	s_Open			= true;
	s_NeedReload	= true;
	s_PendingOpen	= path ? path : "";
	ShowTab(true);
}

void EditorCharacters::SetModelPreview(TModelPreview fn)	{ s_ModelPreview = fn; }

void EditorCharacters::DrawUI()
{
	// portraits of one project are not the next one's; dropped before anything of
	// this frame can reference them
	const xr_string root = EditorProject::Active() ? EditorProject::Root() : "";
	if (root != s_SheetsRoot)
	{
		DropSheets();
		s_SheetsRoot = root;
	}
	if (!s_Open)	return;
	Sync();
	ShowTab(false);
}

bool EditorCharacters::ItemPicker(LPCSTR label, char* buf, u32 size, LPCSTR none)
{
	return ListPicker(label, buf, size, NqPickers::Index(NqPickers::tItem), none);
}

namespace EditorCharacters
{
static void DrawCharactersBody()
{
	// the tab draws before DrawUI: another project's portraits go before this frame can show them
	const xr_string root = EditorProject::Active() ? EditorProject::Root() : "";
	if (root != s_SheetsRoot)
	{
		DropSheets();
		s_SheetsRoot = root;
	}
	if (!EditorProject::Active())
	{
		XUI::EmptyState(XUI::Icon::Character, "No project", "Characters are .npcasset files of the open project.");
		return;
	}
	Sync();
	if (s_OwnStale || s_OwnMounted != EditorGameContent::Mounted())
		Revalidate();

	// the list, a splitter, the form: the list keeps its width, the form takes the rest
	float list_pt = 260.f;
	XUI::Splitter("characters.list", true, &list_pt, 220.f, 400.f, 260.f);
	ImGui::PushStyleColor(ImGuiCol_ChildBg, GrizzlyTheme::U32(Token::Base));
	ImGui::BeginChild("##char_side", ImVec2(XUI::Pt(list_pt), 0.f));
	DrawList();
	ImGui::EndChild();
	ImGui::SameLine();
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(12.f), 0.f));
	ImGui::BeginChild("##char_form", ImVec2(0.f, 0.f), ImGuiChildFlags_AlwaysUseWindowPadding);
	ImGui::PopStyleVar();
	bool changed = false;
	{
		const float column = _min(ImGui::GetContentRegionAvail().x, XUI::Pt(760.f));
		ImGui::BeginChild("##char_column", ImVec2(column, 0.f), ImGuiChildFlags_AutoResizeY);
		if (s_Sel < 0 && !s_Dirty)
		{
			if (XUI::EmptyState(XUI::Icon::Character, "No character selected", "Pick one in the list, or add one with +."))
				NewCharacter();
		}
		else
			changed = DrawForm(column);
		ImGui::EndChild();
	}
	ImGui::EndChild();
	ImGui::PopStyleColor();

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
		s_OwnStale = true;
	}
}
}

void EditorCharacters::McpView(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active())	{ NqUtil::JsonError(out, "no active project"); return; }
	xr_string id, path;
	const bool by_id = NqUtil::ArgString(raw, "select", id) && !id.empty();
	const bool by_path = NqUtil::ArgString(raw, "path", path) && !path.empty();
	// anything to show opens the tab on it; without one this only looks
	if (by_id || by_path || NqUtil::ArgBool(raw, "open", false))
	{
		s_Open = true;
		ShowTab(true);
	}
	// a project path in either slash, any case
	auto same_path = [](xr_string a, xr_string b)
	{
		for (char& c : a) if (c == '/') c = '\\';
		for (char& c : b) if (c == '/') c = '\\';
		return 0 == _stricmp(a.c_str(), b.c_str());
	};
	auto find = [&]()
	{
		int index = -1;
		for (u32 i = 0; i < s_All.size(); ++i)
			if ((by_id && s_All[i].id == id) || (by_path && same_path(NqUtil::ProjectRelative(s_All[i].file.c_str()), path)))
				index = (int)i;
		return index;
	};
	if (s_Open)
	{
		// the files as they are now: a write over MCP may have come after the tab last looked (edits in progress stay)
		ReloadAll();
		if (by_id || by_path)
		{
			int index = find();
			if (index < 0)
			{
				NqUtil::JsonError(out, "no character " + (by_id ? "'" + id + "'" : "at " + path) + " (characters_list names them)");
				return;
			}
			if (index != s_Sel && s_Dirty)
			{
				// the caller decides what happens to edits in progress: MCP never asks
				if (NqUtil::ArgBool(raw, "save", false))
				{
					xr_string err;
					if (!SaveEdited(err))	{ NqUtil::JsonError(out, "the edited character was not saved: " + err); return; }
					ReloadAll();
				}
				else if (NqUtil::ArgBool(raw, "discard", false))
					Discard();
				else
				{
					out = "{\"ok\":false,\"error\":\"unsaved\",\"dirty\":";
					NqUtil::JsonStringArray(out, xr_vector<xr_string>(1, s_Edit.id));
					out += ",\"note\":\"save=true or discard=true settles the edits first\"}";
					return;
				}
				index = find();
			}
			if (index != s_Sel) SelectIndex(index);
		}
		if (s_OwnStale || s_OwnMounted != EditorGameContent::Mounted()) Revalidate();
	}
	out = "{\"ok\":true,\"tab\":\"characters\",\"open\":";
	NqUtil::JsonBool(out, s_Open);
	out += ",\"active\":";
	NqUtil::JsonBool(out, XUI::Shell::IsActive("characters"));
	out += ",\"selected\":";
	NqUtil::JsonString(out, s_Open && (s_Sel >= 0 || s_Dirty) ? s_Edit.id : xr_string());
	out += ",\"file\":";
	NqUtil::JsonPath(out, s_Edit.file.empty() ? "" : NqUtil::ProjectRelative(s_Edit.file.c_str()).c_str());
	out += ",\"dirty\":";
	NqUtil::JsonBool(out, s_Dirty);
	out += NqUtil::Format(",\"count\":%u,\"problems\":", (u32)s_All.size());
	NqUtil::JsonStringArray(out, s_Open ? s_Own : xr_vector<xr_string>());
	out += "}";
}
