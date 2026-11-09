#include "stdafx.h"
#include "EditorWorld.h"
#include "EditorCharacters.h"
#include "EditorProject.h"
#include "EditorGameContent.h"
#include "EditorGameConfigs.h"
#include "EditorGameModes.h"
#include "GrizzlyMCP.h"
#include "Nq/NqUtil.h"
#include "Nq/NqPickers.h"

using EditorMod::SManifest;
using EditorMod::SWorld;

namespace
{
	//--------------------------------------------------------------------------
	// all.spawn, read the way the engine reads it (alife_spawn_registry.cpp):
	//   chunk 1  the spawn graph - one chunk per object, its spawn packet inside
	//   chunk 3  the patrol paths
	//   chunk 4  the game graph - levels, and which level every vertex is on
	// Bounds are checked on every read: a file the editor cannot make sense of
	// leaves the lists empty, it never takes the editor down.
	//--------------------------------------------------------------------------
	struct SBytes
	{
		const u8*	p;
		size_t		size;
		size_t		pos = 0;
		bool		ok = true;

					SBytes		(const u8* data, size_t n) : p(data), size(n) {}
		bool		Need		(size_t n) { if (ok && n > size - pos) ok = false; return ok; }
		template <typename T>
		T			Read		() { T v{}; if (Need(sizeof(T))) { memcpy(&v, p + pos, sizeof(T)); pos += sizeof(T); } return v; }
		void		Skip		(size_t n) { if (Need(n)) pos += n; }
		Fvector		Vec3		() { Fvector v; v.x = Read<float>(); v.y = Read<float>(); v.z = Read<float>(); return v; }
		xr_string	StringZ		()
		{
			const size_t start = pos;
			while (pos < size && p[pos]) ++pos;
			if (pos >= size) { ok = false; return xr_string(); }
			xr_string s;
			s.assign((const char*)p + start, pos - start);
			++pos;
			return s;
		}
	};

	// X-Ray chunks: u32 id (bit 31 = compressed, never used by all.spawn), u32 size, data
	const u32 kChunkCompressed = 0x80000000;

	template <typename F>
	void EachChunk(const u8* data, size_t size, F&& fn)
	{
		size_t pos = 0;
		while (size - pos >= 8)
		{
			u32 id, len;
			memcpy(&id, data + pos, 4);
			memcpy(&len, data + pos + 4, 4);
			pos += 8;
			if (len > size - pos) return;
			if (!(id & kChunkCompressed) && !fn(id, data + pos, size_t(len))) return;
			pos += len;
		}
	}

	bool FindChunk(const u8* data, size_t size, u32 want, const u8*& out, size_t& out_size)
	{
		bool found = false;
		EachChunk(data, size, [&](u32 id, const u8* d, size_t n)
		{
			if (id != want) return true;
			out = d; out_size = n; found = true;
			return false;
		});
		return found;
	}

	struct SGame
	{
		xr_string						key;		// game root + configs the lists were read for
		bool							read = false;
		bool							ok = false;
		xr_string						error;
		xr_vector<EditorWorld::SLevel>	levels;
		xr_vector<EditorWorld::STransition>	transitions;
		xr_set<xr_string>				patrols;
		xr_vector<xr_string>			patrol_list;	// the same, sorted, for pickers
		xr_map<xr_string, xr_string>	captions;
	};
	SGame s_Game;

	xr_string GameKey()
	{
		if (!EditorProject::Active() || !EditorProject::GameLinked()) return xr_string();
		xr_string k = EditorProject::GameRoot();
		k += "|";
		k += EditorGameConfigs::ActiveFingerprint() ? EditorGameConfigs::ActiveFingerprint() : "";
		return k;
	}

	// a section the linked game spawns as a level changer (class LVL_CHNG)
	bool IsLevelChangerSection(const xr_string& section, xr_map<xr_string, bool>& known)
	{
		auto it = known.find(section);
		if (it != known.end()) return it->second;
		bool is = false;
		if (pSettings && pSettings->section_exist(section.c_str()) && pSettings->line_exist(section.c_str(), "class"))
			is = 0 == _stricmp(pSettings->r_string(section.c_str(), "class"), "LVL_CHNG");
		else
			is = 0 == strncmp(section.c_str(), "level_changer", 13);
		known[section] = is;
		return is;
	}

	// One spawn packet: the header CSE_Abstract::Spawn_Read reads, then - for a level
	// changer only - the state chain object -> shape -> restrictor -> changer.
	void ReadSpawnPacket(const u8* data, size_t size, const xr_vector<u8>& vertex_level,
						 const xr_map<u8, xr_string>& level_names, xr_map<xr_string, bool>& known)
	{
		SBytes b(data, size);
		const u16 packet = b.Read<u16>();
		if (!b.ok || packet > size - 2) return;
		SBytes s(data + 2, packet);
		if (s.Read<u16>() != 1 /*M_SPAWN*/) return;
		const xr_string section = s.StringZ();
		if (!s.ok || !IsLevelChangerSection(section, known)) return;
		const xr_string name = s.StringZ();
		s.Skip(2);								// game id, respawn point
		const Fvector position = s.Vec3();
		s.Vec3();								// angle
		s.Skip(8);								// respawn time, id, parent, phantom
		const u16 flags = s.Read<u16>();
		u16 version = 0;
		if (flags & (1 << 5) /*M_SPAWN_VERSION*/) version = s.Read<u16>();
		// older formats predate every game this editor links to
		if (!s.ok || version < 112) return;
		if (version > 120) s.Skip(2);			// game type
		s.Skip(2);								// script version
		const u16 client_data = s.Read<u16>();
		s.Skip(client_data);
		s.Skip(2);								// spawn id
		s.Skip(2);								// state size
		// CSE_ALifeObject
		const u16 graph_id = s.Read<u16>();
		s.Skip(4 + 4 + 4 + 4);					// distance, direct control, node, flags
		s.StringZ();							// custom data
		s.Skip(4 + 4);							// story id, spawn story id
		// CSE_Shape
		const u8 shapes = s.Read<u8>();
		for (u8 i = 0; i < shapes && s.ok; ++i)
		{
			const u8 type = s.Read<u8>();
			s.Skip(type == 0 ? 16 : 48);		// sphere | box (r_matrix: four vectors)
		}
		s.Skip(1);								// restrictor type
		// CSE_ALifeLevelChanger
		s.Skip(2 + 4 + 12 + 12);				// next vertex, next node, next position, angles
		const xr_string to_level = s.StringZ();
		if (!s.ok) return;

		EditorWorld::STransition t;
		t.name		= name;
		t.to_level	= to_level;
		t.position	= position;
		if (graph_id < vertex_level.size())
		{
			auto lvl = level_names.find(vertex_level[graph_id]);
			if (lvl != level_names.end()) t.level = lvl->second;
		}
		s_Game.transitions.push_back(t);
	}

	void ParseSpawn(const u8* bytes, size_t size)
	{
		// ---- chunk 4: the game graph -----------------------------------------
		xr_map<u8, xr_string> level_names;
		xr_vector<u8> vertex_level;
		const u8* gg = 0;
		size_t gg_size = 0;
		if (!FindChunk(bytes, size, 4, gg, gg_size))
		{
			s_Game.error = "all.spawn has no game graph (chunk 4)";
			return;
		}
		{
			SBytes h(gg, gg_size);
			h.Skip(1);									// version
			const u16 vertices = h.Read<u16>();
			h.Skip(4 + 4 + 16);							// edges, death points, guid
			const u8 levels = h.Read<u8>();
			for (u8 i = 0; i < levels && h.ok; ++i)
			{
				const xr_string name = h.StringZ();
				h.Vec3();								// offset
				const u8 id = h.Read<u8>();
				h.StringZ();							// section
				h.Skip(16);								// guid
				if (h.ok) level_names[id] = name;
			}
			// CGameVertex, packed: local point, global point, level:8|node:24, ...
			const size_t kVertex = 42;
			if (h.ok && vertices && size_t(vertices) * kVertex <= gg_size - h.pos)
			{
				vertex_level.resize(vertices);
				for (u32 v = 0; v < vertices; ++v)
					vertex_level[v] = gg[h.pos + v * kVertex + 24];
			}
			if (!h.ok)
			{
				s_Game.error = "the game graph of all.spawn could not be read";
				return;
			}
		}
		for (const auto& [id, name] : level_names)
		{
			EditorWorld::SLevel l;
			l.name = name;
			s_Game.levels.push_back(l);
		}

		// ---- chunk 3: the game's patrol paths --------------------------------
		const u8* pp = 0;
		size_t pp_size = 0;
		const u8* list = 0;
		size_t list_size = 0;
		if (FindChunk(bytes, size, 3, pp, pp_size) && FindChunk(pp, pp_size, 1, list, list_size))
			EachChunk(list, list_size, [](u32, const u8* d, size_t n)
			{
				const u8* name = 0;
				size_t name_size = 0;
				if (FindChunk(d, n, 0, name, name_size))
				{
					SBytes b(name, name_size);
					const xr_string s = b.StringZ();
					if (b.ok && !s.empty()) s_Game.patrols.insert(s);
				}
				return true;
			});

		// ---- chunk 1: every object; the level changers are kept --------------
		const u8* graph = 0;
		size_t graph_size = 0;
		const u8* objects = 0;
		size_t objects_size = 0;
		if (!FindChunk(bytes, size, 1, graph, graph_size) || !FindChunk(graph, graph_size, 1, objects, objects_size))
		{
			s_Game.error = "all.spawn has no objects (chunk 1)";
			return;
		}
		xr_map<xr_string, bool> known;
		EachChunk(objects, objects_size, [&](u32, const u8* d, size_t n)
		{
			const u8* wrapper = 0;
			size_t wrapper_size = 0;
			const u8* spawn = 0;
			size_t spawn_size = 0;
			if (FindChunk(d, n, 1, wrapper, wrapper_size) && FindChunk(wrapper, wrapper_size, 0, spawn, spawn_size))
				ReadSpawnPacket(spawn, spawn_size, vertex_level, level_names, known);
			return true;
		});
		s_Game.ok = true;
	}

	void EnsureGame()
	{
		const xr_string key = GameKey();
		if (s_Game.read && s_Game.key == key) return;
		s_Game = SGame();
		s_Game.read = true;
		s_Game.key = key;
		if (key.empty()) { s_Game.error = "no game is linked"; return; }

		xr_string err;
		if (!EditorGameContent::EnsureMounted(err)) { s_Game.error = err; return; }
		const int idx = EditorGameContent::Find("spawns\\all.spawn");
		if (idx < 0) { s_Game.error = "the linked game has no spawns\\all.spawn"; return; }
		u32 size = 0;
		u8* bytes = EditorGameContent::ReadBytes(idx, size);
		if (!bytes) { s_Game.error = "spawns\\all.spawn could not be read"; return; }
		ParseSpawn(bytes, size);
		EditorGameContent::FreeBytes(bytes);
		s_Game.patrol_list.assign(s_Game.patrols.begin(), s_Game.patrols.end());

		// what the player reads: game_levels.ltx through the string table
		const xr_vector<NqPickers::SEntry>& named = NqPickers::Index(NqPickers::tLevel);
		for (u32 i = 0; i < named.size(); ++i)
			if (!named[i].name.empty()) s_Game.captions[named[i].id] = named[i].name;
		for (EditorWorld::SLevel& l : s_Game.levels)
		{
			auto c = s_Game.captions.find(l.name);
			if (c != s_Game.captions.end()) l.caption = c->second;
		}
		std::sort(s_Game.levels.begin(), s_Game.levels.end(),
			[](const EditorWorld::SLevel& a, const EditorWorld::SLevel& b) { return a.name < b.name; });
		std::sort(s_Game.transitions.begin(), s_Game.transitions.end(),
			[](const EditorWorld::STransition& a, const EditorWorld::STransition& b)
			{ return a.level != b.level ? a.level < b.level : a.name < b.name; });
		if (s_Game.ok)
			Msg("* world: %u level(s), %u level changer(s), %u patrol path(s) in the linked game",
				(u32)s_Game.levels.size(), (u32)s_Game.transitions.size(), (u32)s_Game.patrols.size());
		else
			Msg("! world: %s", s_Game.error.c_str());
	}

	bool Contains(const xr_vector<xr_string>& v, const xr_string& s)
	{
		return std::find(v.begin(), v.end(), s) != v.end();
	}

	void Toggle(xr_vector<xr_string>& v, const xr_string& s, bool on)
	{
		auto it = std::find(v.begin(), v.end(), s);
		if (on && it == v.end())		v.push_back(s);
		else if (!on && it != v.end())	v.erase(it);
	}

	u32 ChoiceCount(const SWorld& w)
	{
		u32 n = 0;
		for (u32 i = 0; i < EditorMod::wrCount; ++i)
			if (w.player_choice[i]) ++n;
		return n;
	}

	// what closes a transition at the start of a game, in the engine's order
	// (XmsWorld::TransitionOpen): the changer itself, its level, the area
	LPCSTR ClosedBy(const SWorld& w, const EditorWorld::STransition& t)
	{
		if (Contains(w.locked, t.name))								return "locked";
		if (Contains(w.locked_levels, t.level))						return "level";
		if (!w.area.empty() && !Contains(w.area, t.to_level))		return "area";
		return "";
	}

	// ---- the open scene --------------------------------------------------------
	EditorWorld::TSceneStarts	s_SceneStarts	= 0;
	EditorWorld::TPlaceStart	s_PlaceStart	= 0;
	EditorWorld::TFocusObject	s_FocusObject	= 0;

	bool ParseVec(const xr_string& text, Fvector& v)
	{
		return 3 == sscanf(text.c_str(), "%f , %f , %f", &v.x, &v.y, &v.z);
	}

	// op = start entries of one spawn layer file
	void ReadStarts(LPCSTR file, xr_vector<EditorWorld::SStart>& out)
	{
		IReader* F = FS.r_open(file);
		if (!F) return;
		xr_string text;
		text.assign((const char*)F->pointer(), F->length());
		FS.r_close(F);
		xr_string section;
		xr_map<xr_string, xr_string> keys;
		auto flush = [&]()
		{
			if (0 == section.compare(0, 4, "obj:") && keys["op"] == "start")
			{
				EditorWorld::SStart s;
				s.name		= section.substr(4);
				s.level		= keys["level"];
				s.source	= NqUtil::ProjectRelative(file);
				s.position.set(0, 0, 0);
				s.direction.set(0, 0, 0);
				ParseVec(keys["position"], s.position);
				ParseVec(keys["direction"], s.direction);
				out.push_back(s);
			}
			keys.clear();
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
			if (line.empty()) continue;
			if (line[0] == '[')
			{
				flush();
				const size_t close = line.find(']');
				section = line.substr(1, close == xr_string::npos ? xr_string::npos : close - 1);
				continue;
			}
			const size_t eq = line.find('=');
			if (eq != xr_string::npos)
				keys[NqUtil::Trim(line.substr(0, eq))] = NqUtil::Trim(line.substr(eq + 1));
		}
		flush();
	}

	//--------------------------------------------------------------------------
	// the World Rules tab
	//--------------------------------------------------------------------------
	using Token = GrizzlyTheme::ColorToken;

	bool			s_Open			= false;
	SWorld			s_World;				// what mod.ltx says, reloaded when it changes
	SManifest		s_Manifest;
	bool			s_HaveManifest	= false;
	u32				s_WorldGen		= u32(-1);
	xr_string		s_WorldRoot;
	xr_string		s_Status;				// the last save's failure
	xr_string		s_TransLevel;			// the level whose transitions are listed
	char			s_TransFilter[64]	= {};
	char			s_Search[64]	= {};	// the level picker's
	bool			s_DrawerOpen	= false;	// the problems drawer
	u32				s_DrawerErrors	= 0;	// errors the drawer last saw: a new one opens it
	// what the tab shows about the project, refreshed a second apart: the spawn layer
	// is files, and a tab must not walk them every frame
	xr_vector<EditorWorld::SStart>	s_ShownStarts;
	xr_vector<xr_string>			s_ShownProblems;
	xr_string						s_SceneLevel;
	u32								s_CheckedAt		= 0;

	struct SRuleText { LPCSTR caption; LPCSTR hint; };
	const SRuleText kRules[EditorMod::wrCount] =
	{
		{ "Original characters",	"Traders, story characters and every squad the game placed by name (a story id). Off: they are not in the world at all." },
		{ "Random stalkers",		"The simulation's stalker squads: the ones placed at the start and the ones smart terrains bring back." },
		{ "Mutants",				"The simulation's monster squads, placed at the start and respawned." },
		{ "Jobs from NPCs",			"\"Got any work?\" - the jobs squad leaders hand out: fetch, kill, escort. Off: the offer is not in their dialogs." },
		{ "Story quests",			"Tasks of the named characters and the storylines. Off: their offers are not in the dialogs." },
	};

	// every edit is saved at once, like Exclude from Build: the manifest is re-read
	// first, so the Mod Settings tab's own edits are never written over
	bool SaveWorld(const SWorld& w, xr_string& err)
	{
		SManifest m;
		if (!EditorProject::Active())						{ err = "no active project"; return false; }
		if (!EditorMod::Load(EditorProject::Root(), m))		{ err = "mod.ltx not found in the project"; return false; }
		m.world = w;
		if (!EditorMod::Save(EditorProject::Root(), m))		{ err = "can't write mod.ltx"; return false; }
		return true;
	}

	void Commit()
	{
		xr_string err;
		s_Status = SaveWorld(s_World, err) ? xr_string() : "Not saved: " + err;
		s_CheckedAt = 0;
	}

	// the mode's start kit as mod.ltx has it, saved the same way
	EditorMod::SStartKit	s_Kit;

	bool SaveKit(const EditorMod::SStartKit& k, xr_string& err)
	{
		SManifest m;
		if (!EditorProject::Active())						{ err = "no active project"; return false; }
		if (!EditorMod::Load(EditorProject::Root(), m))		{ err = "mod.ltx not found in the project"; return false; }
		m.start_kit = k;
		if (!EditorMod::Save(EditorProject::Root(), m))		{ err = "can't write mod.ltx"; return false; }
		return true;
	}

	void CommitKit()
	{
		xr_string err;
		s_Status = SaveKit(s_Kit, err) ? xr_string() : "Not saved: " + err;
		s_CheckedAt = 0;
	}

	xr_string LevelLabel(const xr_string& level)
	{
		const xr_string caption = EditorWorld::Caption(level.c_str());
		return caption == level ? level : caption + "  (" + level + ")";
	}

	// "W203 world: locked names 'x', ..." -> "Locked names 'x', ...". The code is for MCP and the log; the author
	// reads the sentence.
	xr_string ProblemLine(const xr_string& problem)
	{
		const size_t colon = problem.find(": ");
		xr_string text = colon == xr_string::npos ? problem : problem.substr(colon + 2);
		if (!text.empty() && text[0] >= 'a' && text[0] <= 'z')
			text[0] = char(text[0] - 'a' + 'A');
		if (!text.empty() && !strchr(".!?", text.back()))
			text += '.';
		return text;
	}

	// A level of the linked game, picked from a searchable list: the ones listed already are left out.
	bool XUI_LocalLevelPicker(LPCSTR popup, const xr_vector<xr_string>& taken, xr_string& picked)
	{
		bool done = false;
		if (!ImGui::BeginPopup(popup)) return false;
		const float width = XUI::Em(24.f);
		if (ImGui::IsWindowAppearing())
		{
			s_Search[0] = 0;
			ImGui::SetKeyboardFocusHere();
		}
		ImGui::SetNextItemWidth(width);
		XUI::SearchField("##search", s_Search, sizeof(s_Search), "Search levels");
		const xr_vector<EditorWorld::SLevel>& levels = EditorWorld::Levels();
		if (levels.empty())
			XUI::StatusText(XUI::Tone::Muted, "Link a game to pick its levels.");
		else
		{
			ImGui::BeginChild("##levels", ImVec2(width, XUI::Pt(280.f)));
			int shown = 0;
			for (const EditorWorld::SLevel& l : levels)
			{
				if (Contains(taken, l.name)) continue;
				const xr_string label = l.caption.empty() ? l.name : l.name + "  -  " + l.caption;
				if (!XUI::Match(s_Search, label.c_str())) continue;
				++shown;
				if (ImGui::Selectable(label.c_str()))
				{
					picked = l.name;
					done = true;
					ImGui::CloseCurrentPopup();
				}
			}
			if (!shown)
				XUI::StatusText(XUI::Tone::Muted, "No level matches.");
			ImGui::EndChild();
		}
		ImGui::EndPopup();
		return done;
	}

	// the levels of a list as chips, then Add Level...; true when the list changed
	bool DrawLevelChips(LPCSTR id, xr_vector<xr_string>& v)
	{
		bool changed = false;
		ImGui::PushID(id);
		if (!v.empty())
		{
			XUI::Strings shown;
			for (const xr_string& level : v)
				shown.push_back(LevelLabel(level));
			const int removed = XUI::Chips("##chips", shown, true);
			if (removed >= 0)
			{
				v.erase(v.begin() + removed);
				changed = true;
			}
		}
		// names the game does not have: kept, said
		xr_string unknown;
		if (EditorWorld::Available())
			for (const xr_string& level : v)
				if (!EditorWorld::LevelExists(level.c_str()))
					unknown += (unknown.empty() ? "" : ", ") + level;
		if (!unknown.empty())
			XUI::StatusText(XUI::Tone::Warning, ("Not in the linked game: " + unknown + ".").c_str());
		if (XUI::Button("Add Level...", XUI::Kind::Secondary, EditorWorld::Available(), "Link a game to pick its levels"))
			XUI::Shell::OpenPopupAt("pick_level", ImVec2(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y + XUI::Pt(2.f)));
		xr_string picked;
		if (XUI_LocalLevelPicker("pick_level", v, picked))
		{
			v.push_back(picked);
			changed = true;
		}
		ImGui::PopID();
		return changed;
	}

	// Two ways, one list: the first segment empties it (everywhere), the second lets levels be picked.
	bool Scope(LPCSTR id, LPCSTR everywhere, LPCSTR only, xr_vector<xr_string>& v, bool& limited)
	{
		bool changed = false;
		ImGui::PushID(id);
		const char* labels[] = { everywhere, only };
		int pick = limited ? 1 : 0;
		if (XUI::Segmented("##scope", &pick, labels, 2))
		{
			limited = pick == 1;
			if (!limited && !v.empty())
			{
				v.clear();
				changed = true;
			}
		}
		if (limited)
		{
			ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
			changed |= DrawLevelChips("levels", v);
		}
		ImGui::PopID();
		return changed;
	}

	xr_string ModeText(const SManifest& m)
	{
		if (!m.provides_modes.empty())
			return "These rules shape the module's own mode '" + (m.provides_modes[0].title.empty() ? m.provides_modes[0].id : m.provides_modes[0].title) + "'.";
		if (!m.mode.empty() && m.mode != "*")
		{
			for (const EditorGameModes::SMode& mode : EditorGameModes::All())
				if (mode.id == m.mode) return "These rules shape the mode '" + mode.caption + "'.";
			return "These rules shape the mode '" + m.mode + "'.";
		}
		if (0 == _stricmp(m.target.c_str(), "default")) return "These rules shape every new game: the module targets the plain game.";
		if (m.mode == "*") return "These rules shape every new game: the module targets every mode.";
		return "No game mode is chosen yet: pick one in Mod Settings, Game Mode.";
	}

	bool s_OpenMore = false;	// over MCP: what it asks for is under More
	bool s_AreaLimited = false, s_LevelsLimited = false;

	// a checkbox in the middle of its table cell
	bool CentredCheckbox(LPCSTR id, bool* v)
	{
		const float box = ImGui::GetFrameHeight();
		ImGui::SetCursorPosX(ImGui::GetCursorPosX() + _max(0.f, (ImGui::GetContentRegionAvail().x - box) * .5f));
		return ImGui::Checkbox(id, v);
	}

	void DrawRules()
	{
		const u32 choices = ChoiceCount(s_World);
		const ImGuiTableFlags flags = ImGuiTableFlags_RowBg | ImGuiTableFlags_BordersInnerH | ImGuiTableFlags_PadOuterX;
		if (ImGui::BeginTable("##rules", 3, flags))
		{
			ImGui::TableSetupColumn("Rule", ImGuiTableColumnFlags_WidthStretch);
			ImGui::TableSetupColumn("Keep", ImGuiTableColumnFlags_WidthFixed, XUI::Em(5.f));
			ImGui::TableSetupColumn("Player Decides", ImGuiTableColumnFlags_WidthFixed, XUI::Em(8.f));
			XUI::TableHeaders(0x6);
			for (u32 i = 0; i < EditorMod::wrCount; ++i)
			{
				ImGui::PushID((int)i);
				ImGui::TableNextRow(0, XUI::Pt(26.f));
				ImGui::TableNextColumn();
				ImGui::AlignTextToFramePadding();
				XUI::TextEllipsis(kRules[i].caption);
				XUI::Tip((xr_string(kRules[i].hint) + "\nmod.ltx: " + EditorMod::WorldRuleKey(i)).c_str());
				ImGui::TableNextColumn();
				bool keep = s_World.keep[i];
				if (CentredCheckbox("##keep", &keep))
				{
					s_World.keep[i] = keep;
					Commit();
				}
				XUI::Tip(keep ? "Kept in the mode" : "Taken out of the mode");
				ImGui::TableNextColumn();
				bool choice = s_World.player_choice[i];
				ImGui::BeginDisabled(!choice && choices >= EditorWorld::kMaxPlayerChoices);
				if (CentredCheckbox("##choice", &choice))
				{
					s_World.player_choice[i] = choice;
					Commit();
				}
				ImGui::EndDisabled();
				XUI::Tip(!choice && choices >= EditorWorld::kMaxPlayerChoices
					? "The new-game screen has room for three: clear another one first."
					: "A checkbox on the new-game screen, set to what Keep says: the player may change it.");
				ImGui::PopID();
			}
			ImGui::EndTable();
		}
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
		XUI::Prose(NqUtil::Format("%u of %u rules offered on the new-game screen.", choices, EditorWorld::kMaxPlayerChoices).c_str());
		bool all = true;
		for (u32 i = 0; i < EditorMod::wrCount; ++i) all &= s_World.keep[i];
		if (XUI::Button("Keep Everything", XUI::Kind::Secondary, !all, "Everything is kept already"))
		{
			for (u32 i = 0; i < EditorMod::wrCount; ++i) s_World.keep[i] = true;
			Commit();
		}
	}

	void DrawAppliesOn()
	{
		XUI::Prose("Where the five rules hold. The rest of the map keeps the original game.");
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
		if (Scope("levels", "Every Level", "These Levels", s_World.levels, s_LevelsLimited)) Commit();
	}

	void DrawArea()
	{
		XUI::Prose("A transition out of the area stays closed. A quest can open it or grow the area.");
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
		if (Scope("area", "Anywhere", "Only These Levels", s_World.area, s_AreaLimited)) Commit();
	}

	void DrawTransitions(const xr_string& scene_level)
	{
		const xr_vector<EditorWorld::STransition>& all = EditorWorld::Transitions();
		if (!EditorWorld::Available())
		{
			XUI::StatusText(XUI::Tone::Muted, EditorWorld::Error()[0] ? EditorWorld::Error() : "Link a game to list its transitions.");
			return;
		}
		// the scene's level first, then the first of the area, then the first with any
		if (s_TransLevel.empty())
		{
			if (!scene_level.empty())			s_TransLevel = scene_level;
			else if (!s_World.area.empty())		s_TransLevel = s_World.area[0];
			else if (!all.empty())				s_TransLevel = all[0].level;
		}
		if (XUI::BeginPropertyGrid("world", .25f))
		{
			if (XUI::PropertyRow("Level", "The level whose transitions are listed: the linked game's levels that have any."))
			{
				ImGui::SetNextItemWidth(_min(ImGui::CalcItemWidth(), XUI::Em(24.f)));
				if (ImGui::BeginCombo("##trans_level", LevelLabel(s_TransLevel).c_str(), ImGuiComboFlags_HeightLarge))
				{
					for (const EditorWorld::SLevel& l : EditorWorld::Levels())
					{
						u32 n = 0;
						for (const EditorWorld::STransition& t : all) if (t.level == l.name) ++n;
						if (!n) continue;
						const xr_string label = NqUtil::Format("%s  (%u)", LevelLabel(l.name).c_str(), n);
						if (ImGui::Selectable(label.c_str(), l.name == s_TransLevel)) s_TransLevel = l.name;
					}
					ImGui::EndCombo();
				}
			}
			if (XUI::PropertyRow("Whole Level", "Every transition on this level starts closed; quests open them one by one."))
			{
				bool level_locked = Contains(s_World.locked_levels, s_TransLevel);
				if (ImGui::Checkbox("Close every transition on this level", &level_locked))
				{
					Toggle(s_World.locked_levels, s_TransLevel, level_locked);
					Commit();
				}
			}
			XUI::EndPropertyGrid();
		}
		ImGui::SetNextItemWidth(_min(ImGui::GetContentRegionAvail().x, XUI::Em(18.f)));
		XUI::SearchField("##trans_filter", s_TransFilter, sizeof(s_TransFilter), "Filter transitions");

		const ImGuiTableFlags flags = ImGuiTableFlags_RowBg | ImGuiTableFlags_BordersInnerH | ImGuiTableFlags_PadOuterX;
		int shown = 0;
		if (ImGui::BeginTable("##trans", 3, flags))
		{
			ImGui::TableSetupColumn("Transition", ImGuiTableColumnFlags_WidthStretch, 1.f);
			ImGui::TableSetupColumn("Leads To", ImGuiTableColumnFlags_WidthStretch, 1.f);
			ImGui::TableSetupColumn("Closed", ImGuiTableColumnFlags_WidthFixed, XUI::Em(5.f));
			XUI::TableHeaders(0x4);
			for (const EditorWorld::STransition& t : all)
			{
				if (t.level != s_TransLevel) continue;
				const xr_string to = EditorWorld::Caption(t.to_level.c_str());
				if (!XUI::Match(s_TransFilter, (t.name + " " + to + " " + t.to_level).c_str())) continue;
				++shown;
				ImGui::PushID(t.name.c_str());
				ImGui::TableNextRow(0, XUI::Pt(26.f));
				ImGui::TableNextColumn();
				ImGui::AlignTextToFramePadding();
				XUI::TextEllipsis(t.name.c_str());
				XUI::Tip(NqUtil::Format("%s\nat %.1f, %.1f, %.1f", t.name.c_str(), t.position.x, t.position.y, t.position.z).c_str());
				ImGui::TableNextColumn();
				ImGui::AlignTextToFramePadding();
				XUI::TextEllipsis(to.c_str());
				XUI::Tip(t.to_level.c_str());
				ImGui::TableNextColumn();
				const LPCSTR by = ClosedBy(s_World, t);
				bool closed = by[0] != 0;
				const bool own = 0 == xr_strcmp(by, "locked") || !by[0];
				ImGui::BeginDisabled(!own);
				if (CentredCheckbox("##closed", &closed))
				{
					Toggle(s_World.locked, t.name, closed);
					Commit();
				}
				ImGui::EndDisabled();
				XUI::Tip(0 == xr_strcmp(by, "level") ? "Closed with the whole level" :
						 0 == xr_strcmp(by, "area") ? "Closed: it leads out of the playable area" :
						 closed ? "Starts closed: a quest opens it" : "Open at the start of a game");
				ImGui::PopID();
			}
			ImGui::EndTable();
		}
		if (!shown)
			XUI::StatusText(XUI::Tone::Muted, s_TransFilter[0] ? "No transition matches the filter." : "This level has no transitions.");
		// names the game does not have: kept, shown, removable
		for (u32 i = 0; i < s_World.locked.size(); ++i)
		{
			if (EditorWorld::FindTransition(s_World.locked[i].c_str())) continue;
			ImGui::PushID((int)i);
			const bool remove = XUI::IconButton("##forget", XUI::Icon::Close, "Take it off the closed list");
			ImGui::SameLine();
			XUI::StatusText(XUI::Tone::Warning, ("Closed, but not in the linked game: " + s_World.locked[i]).c_str());
			ImGui::PopID();
			if (remove)
			{
				s_World.locked.erase(s_World.locked.begin() + i);
				Commit();
				break;
			}
		}
	}

	void DrawStart(const xr_string& scene_level)
	{
		const xr_vector<EditorWorld::SStart>& starts = s_ShownStarts;
		if (starts.empty())
			XUI::Prose("None: a new game starts where the faction's start puts the player.");
		for (u32 i = 0; i < starts.size(); ++i)
		{
			const EditorWorld::SStart& s = starts[i];
			ImGui::PushID((int)i);
			const bool in_scene = s.source == "scene" && s_FocusObject;
			if (in_scene)
			{
				if (XUI::IconButton("##show", XUI::Icon::FrameSelection, "Select it in the scene and frame it"))
					s_FocusObject(s.name.c_str());
				ImGui::SameLine();
			}
			ImGui::AlignTextToFramePadding();
			XUI::TextEllipsis(NqUtil::Format("%s  at %.1f, %.1f, %.1f", LevelLabel(s.level).c_str(), s.position.x, s.position.y, s.position.z).c_str());
			XUI::Tip((s.source + ": " + s.name).c_str());
			ImGui::PopID();
		}
		if (starts.size() > 1)
			XUI::StatusText(XUI::Tone::Error, NqUtil::Format("A module has one Player Start: the build refuses %u.", (u32)starts.size()).c_str());
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
		if (XUI::Button("Place in Scene", XUI::Kind::Secondary, !scene_level.empty() && s_PlaceStart, "Open the level's scene first"))
		{
			xr_string err;
			if (!s_PlaceStart(err)) s_Status = "Not placed: " + err;
			s_CheckedAt = 0;
		}
		XUI::Tip("In front of the camera; the arrow is where the player faces. The Spawn tool has it too: Player Start.");
	}

	// What a new game of the mode begins with, in place of the new-game screen's loadout.
	void DrawStartKit()
	{
		EditorMod::SStartKit& k = s_Kit;
		if (!k.declared)
		{
			XUI::Prose("None: a new game begins with the loadout picked on the new-game screen - the faction's kit and the extra kits.");
			ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
			if (XUI::Button("Set a Starting Kit", XUI::Kind::Secondary))
			{
				// the game's own base kit is where a mode's kit usually starts from
				if (!EditorWorld::GameBaseKit(k))
				{
					k = EditorMod::SStartKit();
					k.declared = true;
					k.money = 0;
				}
				CommitKit();
			}
			XUI::Tip("It starts as the linked game's base kit, the one every faction gets, and changes from there.");
			return;
		}
		XUI::Prose("A new game begins with exactly this: what the new-game screen's loadout and extra kits gave is taken away. The PDA and the bolt always stay.");
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));

		// money: empty leaves it to the new-game screen
		ImGui::AlignTextToFramePadding();
		ImGui::TextUnformatted("Money");
		ImGui::SameLine(XUI::Em(6.f));
		char money[16] = "";
		if (k.money >= 0) sprintf_s(money, "%d", k.money);
		ImGui::SetNextItemWidth(XUI::Em(8.f));
		if (ImGui::InputTextWithHint("##money", "the screen's", money, sizeof(money), ImGuiInputTextFlags_CharsDecimal))
		{
			k.money = money[0] ? _max(0, atoi(money)) : -1;
			CommitKit();
		}
		XUI::Tip("The money a new game begins with. Empty: what the new-game screen gives.\nmod.ltx: money");
		ImGui::SameLine(0.f, XUI::Pt(16.f));
		bool perks = k.perks;
		if (ImGui::Checkbox("Keep the perks picked on the new-game screen", &perks))
		{
			k.perks = perks;
			CommitKit();
		}
		XUI::Tip("Perks are not items: knife master, strong stomach and the rest. Off: a new game begins without them.\nmod.ltx: perks");
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));

		const ImGuiTableFlags flags = ImGuiTableFlags_RowBg | ImGuiTableFlags_BordersInnerH | ImGuiTableFlags_PadOuterX;
		if (k.items.empty())
			XUI::StatusText(XUI::Tone::Muted, "No items: the player begins with the PDA and nothing else.");
		else if (ImGui::BeginTable("##kit", 3, flags))
		{
			ImGui::TableSetupColumn("Item", ImGuiTableColumnFlags_WidthStretch);
			ImGui::TableSetupColumn("Count", ImGuiTableColumnFlags_WidthFixed, XUI::Em(5.f));
			ImGui::TableSetupColumn("##actions", ImGuiTableColumnFlags_WidthFixed, XUI::Pt(24.f));
			XUI::TableHeaders();
			int remove = -1;
			for (u32 i = 0; i < k.items.size(); ++i)
			{
				ImGui::PushID((int)i);
				ImGui::TableNextRow(0, XUI::Pt(26.f));
				ImGui::TableNextColumn();
				char section[128];
				strncpy_s(section, k.items[i].section.c_str(), _TRUNCATE);
				ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
				if (EditorCharacters::ItemPicker("##sect", section, sizeof(section)) && section[0])
				{
					k.items[i].section = section;
					CommitKit();
				}
				ImGui::TableNextColumn();
				ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
				int count = k.items[i].count;
				if (ImGui::InputInt("##count", &count, 0, 0))
				{
					k.items[i].count = _max(1, count);
					CommitKit();
				}
				XUI::Tip("How many of it; ammo counts in boxes.");
				ImGui::TableNextColumn();
				if (XUI::IconButton("##remove", XUI::Icon::Delete, "Take the row out"))	remove = (int)i;
				ImGui::PopID();
			}
			ImGui::EndTable();
			if (remove >= 0)
			{
				k.items.erase(k.items.begin() + remove);
				CommitKit();
			}
		}
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
		// a row is born with its item: the tab saves at once, and a row without one would not survive the save
		char add[128] = "";
		ImGui::SetNextItemWidth(XUI::Em(18.f));
		if (EditorCharacters::ItemPicker("##add", add, sizeof(add), "Add an item...") && add[0])
		{
			bool merged = false;
			for (EditorMod::SKitItem& it : k.items)
				if (it.section == add) { ++it.count; merged = true; break; }
			if (!merged)
			{
				EditorMod::SKitItem it;
				it.section = add;
				k.items.push_back(it);
			}
			CommitKit();
		}
		ImGui::SameLine();
		if (XUI::Button("Game's Base Kit", XUI::Kind::Secondary, NqPickers::Available(), "Link a game to read its kit"))
		{
			EditorMod::SStartKit base;
			if (EditorWorld::GameBaseKit(base))
			{
				base.perks = k.perks;
				k = base;
				CommitKit();
			}
			else
				XUI::Toast(XUI::Tone::Warning, "The linked game has no plugins\\faction_loadouts.ltx to read its base kit from.");
		}
		XUI::Tip("In place of these rows and the money: what every faction starts with in the linked game ([shared] of plugins\\faction_loadouts.ltx).");
		ImGui::SameLine();
		if (XUI::Button("Remove Kit", XUI::Kind::GhostDanger))
		{
			k = EditorMod::SStartKit();
			CommitKit();
		}
		XUI::Tip("No kit of the mode's own: the new-game screen's loadout decides again.");
	}

	// Take everything of the original game out: reversible, but it is five decisions at once, so it asks.
	void AskEmptyWorld()
	{
		XUI::Strings kept;
		for (u32 i = 0; i < EditorMod::wrCount; ++i)
			if (s_World.keep[i]) kept.push_back(kRules[i].caption);
		if (kept.empty())
		{
			XUI::Toast(XUI::Tone::Info, "The world is empty already: the mode keeps nothing of the original game.");
			return;
		}
		XUI::ConfirmSpec spec;
		spec.id = "empty_world";
		spec.title = "Empty World";
		spec.headline = "Take everything of the original game out of this mode?";
		spec.items = kept;
		spec.consequence = "Your own characters and quests stay. It is saved to mod.ltx at once; Keep Everything brings it back.";
		spec.action = "Empty World";
		XUI::Confirm(spec, [](XUI::Answer answer)
		{
			if (answer != XUI::Answer::Yes) return;
			for (u32 i = 0; i < EditorMod::wrCount; ++i) s_World.keep[i] = false;
			Commit();
		});
	}

	void CountProblems(u32& errors, u32& warnings)
	{
		errors = warnings = 0;
		for (const xr_string& p : s_ShownProblems)
			++(p[0] == 'E' ? errors : warnings);
		if (!s_Status.empty()) ++errors;
	}

	// The problems drawer at the bottom of the tab: a header that says how many, and the list
	// under it while open. A new error opens it.
	float DrawerHeight(float avail)
	{
		const float header = XUI::Pt(28.f);
		return s_DrawerOpen ? header + _min(XUI::Pt(180.f), avail * .4f) : header;
	}

	void XUI_LocalProblemsDrawer(float height)
	{
		u32 errors = 0, warnings = 0;
		CountProblems(errors, warnings);
		const ImVec2 at = ImGui::GetCursorScreenPos();
		const float width = _max(1.f, ImGui::GetContentRegionAvail().x);
		const float header = XUI::Pt(28.f);
		ImDrawList* dl = ImGui::GetWindowDrawList();
		if (ImGui::InvisibleButton("##problems_header", ImVec2(width, header)))
			s_DrawerOpen = !s_DrawerOpen;
		const bool hovered = ImGui::IsItemHovered();
		XUI::Tip(s_DrawerOpen ? "Hide the problems" : "Show the problems");
		dl->AddRectFilled(at, ImVec2(at.x + width, at.y + header), GrizzlyTheme::U32(hovered ? Token::Hover : Token::Raised), XUI::Pt(4.f));
		const float cy = at.y + header * .5f;
		XUI::DrawIcon(dl, s_DrawerOpen ? XUI::Icon::ChevronDown : XUI::Icon::ChevronUp, ImVec2(at.x + XUI::Pt(6.f), cy - XUI::Pt(6.f)), 12.f,
			GrizzlyTheme::U32(Token::TextSecondary));
		XUI::DrawIcon(dl, XUI::Icon::Problems, ImVec2(at.x + XUI::Pt(24.f), cy - XUI::Pt(7.f)), 14.f,
			errors ? XUI::ToneColor(XUI::Tone::Error) : warnings ? XUI::ToneColor(XUI::Tone::Warning) : GrizzlyTheme::U32(Token::TextSecondary));
		const xr_string summary = !errors && !warnings ? xr_string("Problems: none") :
			NqUtil::Format("Problems: %u error%s, %u warning%s", errors, errors == 1 ? "" : "s", warnings, warnings == 1 ? "" : "s");
		{
			XUI::FontScope strong(XUI::Font::Strong);
			ImGui::SetCursorScreenPos(ImVec2(at.x + XUI::Pt(44.f), cy - ImGui::GetTextLineHeight() * .5f));
			XUI::TextEllipsis(summary.c_str(), _max(0.f, width - XUI::Pt(52.f)));
		}
		ImGui::SetCursorScreenPos(ImVec2(at.x, at.y + header));
		if (!s_DrawerOpen)
			return;
		ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(12.f), XUI::Pt(8.f)));
		ImGui::BeginChild("##world_problems", ImVec2(0.f, _max(XUI::Pt(24.f), height - header)), ImGuiChildFlags_AlwaysUseWindowPadding);
		ImGui::PopStyleVar();
		if (!s_Status.empty())
			XUI::StatusText(XUI::Tone::Error, s_Status.c_str());
		for (const xr_string& p : s_ShownProblems)
			XUI::StatusText(p[0] == 'E' ? XUI::Tone::Error : XUI::Tone::Warning, ProblemLine(p).c_str());
		if (s_ShownProblems.empty() && s_Status.empty())
			XUI::StatusText(XUI::Tone::Success, "No problems: the build takes these rules as they are.");
		ImGui::EndChild();
	}
}

//------------------------------------------------------------------------------
// the linked game
//------------------------------------------------------------------------------
bool EditorWorld::Available()								{ EnsureGame(); return s_Game.ok; }
LPCSTR EditorWorld::Error()									{ EnsureGame(); return s_Game.error.c_str(); }
const xr_vector<EditorWorld::SLevel>& EditorWorld::Levels()	{ EnsureGame(); return s_Game.levels; }
const xr_vector<EditorWorld::STransition>& EditorWorld::Transitions() { EnsureGame(); return s_Game.transitions; }
void EditorWorld::Invalidate()								{ s_Game.read = false; }

bool EditorWorld::LevelExists(LPCSTR name)
{
	EnsureGame();
	for (const SLevel& l : s_Game.levels)
		if (0 == _stricmp(l.name.c_str(), name ? name : "")) return true;
	return false;
}

const EditorWorld::STransition* EditorWorld::FindTransition(LPCSTR name)
{
	EnsureGame();
	for (const STransition& t : s_Game.transitions)
		if (t.name == (name ? name : "")) return &t;
	return 0;
}

bool EditorWorld::GamePatrolExists(LPCSTR name)
{
	EnsureGame();
	return name && s_Game.patrols.count(xr_string(name)) != 0;
}

const xr_vector<xr_string>& EditorWorld::GamePatrols()
{
	EnsureGame();
	return s_Game.patrol_list;
}

xr_string EditorWorld::Caption(LPCSTR level)
{
	EnsureGame();
	auto it = s_Game.captions.find(xr_string(level ? level : ""));
	return it != s_Game.captions.end() ? it->second : xr_string(level ? level : "");
}

//------------------------------------------------------------------------------
// the project
//------------------------------------------------------------------------------
void EditorWorld::SetSceneStarts(TSceneStarts fn)	{ s_SceneStarts = fn; }
void EditorWorld::SetPlaceStart(TPlaceStart fn)		{ s_PlaceStart = fn; }
void EditorWorld::SetFocusObject(TFocusObject fn)	{ s_FocusObject = fn; }

void EditorWorld::Starts(LPCSTR project_root, xr_vector<SStart>& out)
{
	out.clear();
	if (!project_root || !project_root[0]) return;
	xr_string scene_level;
	xr_vector<SStart> scene;
	if (s_SceneStarts) s_SceneStarts(scene_level, scene);

	string_path dir;
	sprintf_s(dir, "%s\\spawn", project_root);
	xr_vector<xr_string> files;
	NqUtil::ListFiles(dir, ".xspawn", files);
	for (const xr_string& f : files)
	{
		xr_vector<SStart> found;
		ReadStarts(f.c_str(), found);
		for (const SStart& s : found)
			if (scene_level.empty() || 0 != _stricmp(s.level.c_str(), scene_level.c_str())) out.push_back(s);
	}
	for (const SStart& s : scene) out.push_back(s);
}

void EditorWorld::Validate(LPCSTR project_root, const SManifest& m, xr_vector<xr_string>& problems)
{
	const SWorld& w = m.world;
	xr_vector<SStart> starts;
	Starts(project_root, starts);
	if (starts.size() > 1)
	{
		xr_string list;
		for (const SStart& s : starts)
		{
			if (!list.empty()) list += ", ";
			list += s.level + ":" + s.name + " (" + s.source + ")";
		}
		problems.push_back("E201 world: " + NqUtil::Format("%u Player Starts - a module has one: ", (u32)starts.size()) + list);
	}
	if (ChoiceCount(w) > kMaxPlayerChoices)
		problems.push_back(NqUtil::Format("E202 world: %u rules offered to the player - the new-game screen has room for %u",
			ChoiceCount(w), kMaxPlayerChoices));

	if (Available())
	{
		auto levels = [&](LPCSTR key, const xr_vector<xr_string>& v)
		{
			for (const xr_string& l : v)
				if (!LevelExists(l.c_str()))
					problems.push_back("W203 world: " + xr_string(key) + " names '" + l + "', a level the linked game does not have");
		};
		levels("levels", w.levels);
		levels("area", w.area);
		levels("locked_levels", w.locked_levels);
		for (const xr_string& t : w.locked)
			if (!FindTransition(t.c_str()))
				problems.push_back("W204 world: locked names '" + t + "', a transition the linked game does not have");
		for (const SStart& s : starts)
			if (!LevelExists(s.level.c_str()))
				problems.push_back("W203 world: the Player Start " + s.name + " stands on '" + s.level + "', a level the linked game does not have");
	}
	if (!w.area.empty())
	{
		if (starts.empty())
			problems.push_back("W206 world: the playable area is limited, but there is no Player Start - "
							   "the player begins at the faction's start, which may lie outside it");
		for (const SStart& s : starts)
			if (!Contains(w.area, s.level))
				problems.push_back("W205 world: the Player Start stands on " + s.level + ", outside the playable area");
	}

	// the start kit: a row the game cannot use is the author's slip; an item the linked game
	// lacks may be one the module brings, so that one only warns (the Characters' W117)
	const EditorMod::SStartKit& k = m.start_kit;
	if (k.declared)
	{
		xr_vector<xr_string> seen;
		for (const EditorMod::SKitItem& it : k.items)
		{
			if (it.section.empty())
			{
				problems.push_back("E207 world: a start kit row names no item");
				continue;
			}
			if (it.count < 1)
				problems.push_back(NqUtil::Format("E207 world: start kit item '%s' has a count of %d - at least 1", it.section.c_str(), it.count));
			if (Contains(seen, it.section))
				problems.push_back("E207 world: start kit item '" + it.section + "' is listed twice - one row says how many");
			seen.push_back(it.section);
			if (NqPickers::Available() && !NqPickers::Known(NqPickers::tItem, it.section.c_str()))
				problems.push_back("W208 world: start kit item '" + it.section + "' is not an item of the linked game - a new game leaves it out unless the module brings it");
		}
	}
}

void EditorWorld::JsonStartKit(const EditorMod::SStartKit& k, xr_string& out)
{
	out += "{\"declared\":";	NqUtil::JsonBool(out, k.declared);
	out += ",\"money\":";		out += k.money >= 0 ? NqUtil::Format("%d", k.money) : xr_string("null");
	out += ",\"perks\":";		NqUtil::JsonBool(out, k.perks);
	out += ",\"items\":[";
	for (u32 i = 0; i < k.items.size(); ++i)
	{
		if (i) out += ",";
		out += "{\"section\":";	NqUtil::JsonString(out, k.items[i].section);
		out += NqUtil::Format(",\"count\":%d}", k.items[i].count);
	}
	out += "]}";
}

bool EditorWorld::GameBaseKit(EditorMod::SStartKit& out)
{
	out = EditorMod::SStartKit();
	xr_string err;
	if (!EditorGameContent::EnsureMounted(err)) return false;
	const int idx = EditorGameContent::Find("configs\\plugins\\faction_loadouts.ltx");
	if (idx < 0) return false;
	u32 size = 0;
	u8* data = EditorGameContent::ReadBytes(idx, size);
	if (!data) return false;
	xr_string text;
	text.assign((const char*)data, size);
	EditorGameContent::FreeBytes(data);

	// [shared] is what the new-game screen starts every faction's loadout from; a money list
	// ("1000,3000", one picked at random) gives its first amount
	bool shared = false;
	size_t pos = 0;
	while (pos < text.size())
	{
		size_t eol = text.find('\n', pos);
		if (eol == xr_string::npos) eol = text.size();
		xr_string line = NqUtil::Trim(text.substr(pos, eol - pos));
		pos = eol + 1;
		const size_t sc = line.find(';');
		if (sc != xr_string::npos) line = NqUtil::Trim(line.substr(0, sc));
		if (line.empty()) continue;
		if (line[0] == '[')
		{
			const size_t close = line.find(']');
			shared = 0 == _stricmp(line.substr(1, close == xr_string::npos ? xr_string::npos : close - 1).c_str(), "shared");
			continue;
		}
		if (!shared) continue;
		xr_string key = line, val;
		const size_t eq = line.find('=');
		if (eq != xr_string::npos) { key = NqUtil::Trim(line.substr(0, eq)); val = NqUtil::Trim(line.substr(eq + 1)); }
		if (key == "money")
		{
			out.money = _max(0, atoi(val.c_str()));
			continue;
		}
		EditorMod::SKitItem it;
		it.section = key;
		it.count = val.empty() ? 1 : _max(1, atoi(val.c_str()));
		out.items.push_back(it);
	}
	out.declared = true;
	return !out.items.empty() || out.money >= 0;
}

void EditorWorld::JsonWorld(const SWorld& w, xr_string& out)
{
	out += "{\"rules\":{";
	for (u32 i = 0; i < EditorMod::wrCount; ++i)
	{
		if (i) out += ",";
		NqUtil::JsonString(out, EditorMod::WorldRuleKey(i));
		out += w.keep[i] ? ":true" : ":false";
	}
	out += "},\"player_choice\":[";
	bool first = true;
	for (u32 i = 0; i < EditorMod::wrCount; ++i)
		if (w.player_choice[i])
		{
			if (!first) out += ",";
			first = false;
			NqUtil::JsonString(out, EditorMod::WorldRuleKey(i));
		}
	out += "],\"levels\":";			NqUtil::JsonStringArray(out, w.levels);
	out += ",\"area\":";			NqUtil::JsonStringArray(out, w.area);
	out += ",\"locked_levels\":";	NqUtil::JsonStringArray(out, w.locked_levels);
	out += ",\"locked\":";			NqUtil::JsonStringArray(out, w.locked);
	out += ",\"declared\":";		NqUtil::JsonBool(out, !w.Empty());
	out += "}";
}

//------------------------------------------------------------------------------
// UI
//------------------------------------------------------------------------------
namespace EditorWorld
{
static void DrawWorldBody();
}

namespace
{
void DrawWorldToolbar(XUI::Toolbar& tb)
{
	tb.Group(2);
	if (tb.Action("reload", XUI::Icon::Refresh, "Reload Game Data", "Read the linked game's levels, transitions and their names again"))
	{
		EditorWorld::Invalidate();
		NqPickers::Invalidate();
		s_CheckedAt = 0;
	}
	if (tb.Action("empty", XUI::Icon::Clear, "Empty World...", s_HaveManifest ? "Take everything of the original game out of this mode" :
		"The project has no mod.ltx yet", s_HaveManifest))
		AskEmptyWorld();
	tb.Group(0);
	tb.Separator();
	tb.Status(s_Status.empty() ? "Saved to mod.ltx as you edit" : s_Status.c_str(), s_Status.empty() ? XUI::Tone::Muted : XUI::Tone::Error);
}

// World Rules is one of the editor's major tabs (XUI::Shell); it saves as it is edited, so it is never dirty
void ShowTab(bool activate)
{
	if (XUI::Shell::IsOpen("world_rules"))
	{
		if (activate) XUI::Shell::Activate("world_rules");
		return;
	}
	XUI::Shell::DocDesc d;
	d.id = "world_rules";
	d.kind = "world_rules";
	d.icon = XUI::Icon::World;
	d.label = [] { return xr_string("World Rules"); };
	d.tooltip = [] { return xr_string("What the mode keeps of the original game - saved to mod.ltx as you edit"); };
	d.alive = [] { return s_Open; };
	d.toolbar = [](XUI::Toolbar& tb) { DrawWorldToolbar(tb); };
	d.body = [] { EditorWorld::DrawWorldBody(); };
	d.on_close = [] { s_Open = false; };
	XUI::Shell::Open(d, activate);
}

// mod.ltx is the one truth: re-read whenever anything saved it; the starts and the problems a second apart
void RefreshWorld()
{
	if (s_WorldGen != EditorMod::ManifestGeneration() || s_WorldRoot != EditorProject::Root())
	{
		const bool new_project = s_WorldRoot != EditorProject::Root();
		s_HaveManifest = EditorMod::Load(EditorProject::Root(), s_Manifest);
		s_World = s_Manifest.world;
		s_Kit = s_Manifest.start_kit;
		s_WorldGen = EditorMod::ManifestGeneration();
		s_WorldRoot = EditorProject::Root();
		s_AreaLimited |= !s_World.area.empty();
		s_LevelsLimited |= !s_World.levels.empty();
		if (new_project)
		{
			s_AreaLimited = !s_World.area.empty();
			s_LevelsLimited = !s_World.levels.empty();
			s_TransLevel.clear();
			s_TransFilter[0] = 0;
			s_DrawerErrors = 0;
		}
		s_CheckedAt = 0;
	}
	const u32 now = ::GetTickCount();
	if (s_CheckedAt && now - s_CheckedAt <= 1000) return;
	s_CheckedAt = now ? now : 1;
	EditorWorld::Starts(EditorProject::Root(), s_ShownStarts);
	s_ShownProblems.clear();
	if (s_HaveManifest)
		EditorWorld::Validate(EditorProject::Root(), s_Manifest, s_ShownProblems);
	s_SceneLevel.clear();
	if (s_SceneStarts)
	{
		xr_vector<EditorWorld::SStart> unused;
		s_SceneStarts(s_SceneLevel, unused);
	}
	// a new error opens the drawer: the author should not have to go looking for it
	u32 errors = 0, warnings = 0;
	CountProblems(errors, warnings);
	if (errors > s_DrawerErrors) s_DrawerOpen = true;
	s_DrawerErrors = errors;
}
}

bool EditorWorld::LevelChips(LPCSTR id, xr_vector<xr_string>& v) { return DrawLevelChips(id, v); }

void EditorWorld::Show()	{ s_Open = true; ShowTab(true); }
bool EditorWorld::IsOpen()	{ return s_Open; }

void EditorWorld::DrawUI()
{
	if (!s_Open) return;
	ShowTab(false);
}

void EditorWorld::DrawWorldBody()
{
	if (!EditorProject::Active())
	{
		XUI::EmptyState(XUI::Icon::World, "No project", "World rules live in the mod.ltx of the open project.");
		return;
	}
	RefreshWorld();
	if (!s_HaveManifest)
	{
		if (XUI::EmptyState(XUI::Icon::World, "No mod.ltx yet", "World rules live in the module's mod.ltx. Mod Settings makes it.", "Open Mod Settings"))
			EditorMod::RequestEditManifest();
		return;
	}
	const xr_string& scene_level = s_SceneLevel;
	// the problems have a bar of their own only when there are any
	u32 errors = 0, warnings = 0;
	CountProblems(errors, warnings);
	const bool any_problem = errors + warnings > 0;
	const float drawer = any_problem ? DrawerHeight(ImGui::GetContentRegionAvail().y) : 0.f;

	// one column of up to 900 pt in the middle, the scrollbar at the right edge of the tab
	ImGui::PushStyleColor(ImGuiCol_ChildBg, GrizzlyTheme::U32(Token::Base));
	ImGui::BeginChild("##world_scroll", ImVec2(0.f, any_problem ? -drawer - ImGui::GetStyle().ItemSpacing.y : 0.f));
	{
		const float avail = ImGui::GetContentRegionAvail().x;
		const float column = _min(avail, XUI::Pt(900.f));
		ImGui::SetCursorPosX(ImGui::GetCursorPosX() + _max(0.f, (avail - column) * .5f));
		ImGui::BeginChild("##world_column", ImVec2(column, 0.f), ImGuiChildFlags_AutoResizeY);
		ImGui::PopStyleColor();
		XUI::Prose(ModeText(s_Manifest).c_str());
		XUI::Tip("A new game reads these rules once and keeps them in its saves. Quests change them while the game runs.");
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(6.f)));
		if (XUI::Section("Original Game", true))
		{
			DrawRules();
			XUI::EndSection();
		}
		if (XUI::Section("Player Start", true))
		{
			DrawStart(scene_level);
			XUI::EndSection();
		}
		if (XUI::Section("Starting Kit", true, !s_Kit.declared ? "The new-game screen's" :
			NqUtil::Format("%u item%s", (u32)s_Kit.items.size(), s_Kit.items.size() == 1 ? "" : "s").c_str()))
		{
			DrawStartKit();
			XUI::EndSection();
		}
		if (s_OpenMore)
		{
			ImGui::GetStateStorage()->SetBool(ImGui::GetID("More##world"), true);
			s_OpenMore = false;
		}
		if (XUI::Section("More##world", false))
		{
			if (XUI::Section("Applies On", true, s_World.levels.empty() ? "Every level" : NqUtil::Format("%u levels", (u32)s_World.levels.size()).c_str()))
			{
				DrawAppliesOn();
				XUI::EndSection();
			}
			if (XUI::Section("Playable Area", true, s_World.area.empty() ? "Anywhere" : NqUtil::Format("%u levels", (u32)s_World.area.size()).c_str()))
			{
				DrawArea();
				XUI::EndSection();
			}
			if (XUI::Section("Transitions", true))
			{
				DrawTransitions(scene_level);
				XUI::EndSection();
			}
			XUI::EndSection();
		}
		ImGui::EndChild();
	}
	ImGui::EndChild();
	if (any_problem)
		XUI_LocalProblemsDrawer(drawer);
}

//------------------------------------------------------------------------------
// MCP
//------------------------------------------------------------------------------
static void JsonStarts(const xr_vector<EditorWorld::SStart>& starts, xr_string& out)
{
	out += "[";
	for (u32 i = 0; i < starts.size(); ++i)
	{
		const EditorWorld::SStart& s = starts[i];
		if (i) out += ",";
		out += "{\"level\":";		NqUtil::JsonString(out, s.level);
		out += ",\"name\":";		NqUtil::JsonString(out, s.name);
		out += ",\"source\":";		NqUtil::JsonString(out, s.source);
		out += NqUtil::Format(",\"position\":[%.3f,%.3f,%.3f],\"direction\":[%.4f,%.4f,%.4f]}",
			s.position.x, s.position.y, s.position.z, s.direction.x, s.direction.y, s.direction.z);
	}
	out += "]";
}

static void WorldAnswer(const SManifest& m, xr_string& out)
{
	out = "{\"ok\":true,\"module\":";
	NqUtil::JsonString(out, m.id);
	out += ",\"mode\":";
	NqUtil::JsonString(out, !m.provides_modes.empty() ? m.provides_modes[0].id : m.mode);
	out += ",\"target\":";
	NqUtil::JsonString(out, m.target);
	out += ",\"world\":";
	EditorWorld::JsonWorld(m.world, out);
	out += ",\"start_kit\":";
	EditorWorld::JsonStartKit(m.start_kit, out);
	xr_vector<EditorWorld::SStart> starts;
	EditorWorld::Starts(EditorProject::Root(), starts);
	out += ",\"starts\":";
	JsonStarts(starts, out);
	xr_vector<xr_string> problems;
	EditorWorld::Validate(EditorProject::Root(), m, problems);
	out += ",\"problems\":";
	NqUtil::JsonStringArray(out, problems);
	out += ",\"game\":{\"available\":";
	NqUtil::JsonBool(out, EditorWorld::Available());
	out += NqUtil::Format(",\"levels\":%u,\"transitions\":%u", (u32)EditorWorld::Levels().size(), (u32)EditorWorld::Transitions().size());
	out += ",\"error\":";
	NqUtil::JsonString(out, EditorWorld::Error());
	out += "}}";
}

void EditorWorld::McpView(LPCSTR raw, xr_string& out)
{
	if (NqUtil::ArgBool(raw, "more", false) || strstr(raw, "\"level\"") || strstr(raw, "\"filter\""))
		s_OpenMore = true;
	if (!EditorProject::Active())	{ NqUtil::JsonError(out, "no active project"); return; }
	xr_string drawer, level, filter;
	const bool has_drawer = NqUtil::ArgString(raw, "drawer", drawer) && !drawer.empty();
	const bool has_level = NqUtil::ArgString(raw, "level", level) && !level.empty();
	const bool has_filter = NqUtil::ArgString(raw, "filter", filter);
	if (has_drawer && drawer != "open" && drawer != "close")	{ NqUtil::JsonError(out, "drawer must be open or close"); return; }
	if (has_level && !LevelExists(level.c_str()))
	{
		NqUtil::JsonError(out, "the linked game has no level '" + level + "' (world_transitions lists them)");
		return;
	}
	// anything to set opens the tab on it; without one this only looks
	if (has_drawer || has_level || has_filter || NqUtil::ArgBool(raw, "open", false))
		Show();
	if (has_drawer)	s_DrawerOpen = drawer == "open";
	if (has_level)	s_TransLevel = level;
	if (has_filter)	strncpy_s(s_TransFilter, filter.c_str(), _TRUNCATE);

	SManifest m;
	xr_vector<xr_string> problems;
	if (EditorMod::Load(EditorProject::Root(), m))
		Validate(EditorProject::Root(), m, problems);
	out = "{\"ok\":true,\"tab\":\"world_rules\",\"open\":";
	NqUtil::JsonBool(out, s_Open);
	out += ",\"active\":";
	NqUtil::JsonBool(out, XUI::Shell::IsActive("world_rules"));
	out += ",\"drawer\":";
	NqUtil::JsonString(out, s_DrawerOpen ? "open" : "close");
	out += ",\"level\":";
	NqUtil::JsonString(out, s_TransLevel);
	out += ",\"filter\":";
	NqUtil::JsonString(out, s_TransFilter);
	out += ",\"status\":";
	NqUtil::JsonString(out, s_Status.empty() ? xr_string("Saved to mod.ltx as you edit") : s_Status);
	out += ",\"problems\":";
	NqUtil::JsonStringArray(out, problems);
	out += "}";
}

void EditorWorld::McpGet(LPCSTR raw, xr_string& out)
{
	SManifest m;
	if (!EditorProject::Active())						{ NqUtil::JsonError(out, "no active project"); return; }
	if (!EditorMod::Load(EditorProject::Root(), m))		{ NqUtil::JsonError(out, "mod.ltx not found in the project"); return; }
	(void)raw;
	WorldAnswer(m, out);
}

// Present or not: a missing field reads as the default, so asking with both
// defaults tells a false from an absent one.
static int ArgTri(LPCSTR raw, LPCSTR field)
{
	const bool a = NqUtil::ArgBool(raw, field, false), b = NqUtil::ArgBool(raw, field, true);
	return a == b ? (a ? 1 : 0) : -1;
}

static bool ArgList(LPCSTR raw, LPCSTR field, xr_vector<xr_string>& out)
{
	xr_string text;
	if (!NqUtil::ArgString(raw, field, text)) return false;
	NqUtil::Split(text.c_str(), ',', out);
	return true;
}

void EditorWorld::McpSet(LPCSTR raw, xr_string& out)
{
	SManifest m;
	if (!EditorProject::Active())						{ NqUtil::JsonError(out, "no active project"); return; }
	if (!EditorMod::Load(EditorProject::Root(), m))		{ NqUtil::JsonError(out, "mod.ltx not found in the project"); return; }
	SWorld w = m.world;

	xr_string preset;
	if (NqUtil::ArgString(raw, "preset", preset))
	{
		if (preset == "stock")		w = SWorld();
		else if (preset == "empty")	for (u32 i = 0; i < EditorMod::wrCount; ++i) w.keep[i] = false;
		else						{ NqUtil::JsonError(out, "preset must be stock (the game as it is) or empty (nothing of the original game)"); return; }
	}
	for (u32 i = 0; i < EditorMod::wrCount; ++i)
	{
		const int v = ArgTri(raw, EditorMod::WorldRuleKey(i));
		if (v >= 0) w.keep[i] = v == 1;
	}
	xr_vector<xr_string> list;
	if (ArgList(raw, "player_choice", list))
	{
		for (u32 i = 0; i < EditorMod::wrCount; ++i) w.player_choice[i] = false;
		for (const xr_string& key : list)
		{
			const u32 rule = EditorMod::WorldRuleFromKey(key.c_str());
			if (rule >= EditorMod::wrCount)
			{
				NqUtil::JsonError(out, "player_choice: unknown rule '" + key + "' (original_npcs, random_stalkers, mutants, npc_quests, story_quests)");
				return;
			}
			w.player_choice[rule] = true;
		}
	}
	if (ChoiceCount(w) > kMaxPlayerChoices)
	{
		NqUtil::JsonError(out, NqUtil::Format("player_choice: the new-game screen has room for %u rules", kMaxPlayerChoices));
		return;
	}
	ArgList(raw, "levels", w.levels);
	ArgList(raw, "area", w.area);
	ArgList(raw, "locked_levels", w.locked_levels);
	ArgList(raw, "locked", w.locked);
	auto each = [&](LPCSTR field, xr_vector<xr_string>& target, bool on)
	{
		xr_vector<xr_string> names;
		if (ArgList(raw, field, names))
			for (const xr_string& n : names) Toggle(target, n, on);
	};
	each("lock", w.locked, true);
	each("unlock", w.locked, false);
	each("lock_level", w.locked_levels, true);
	each("unlock_level", w.locked_levels, false);
	each("area_add", w.area, true);
	each("area_remove", w.area, false);

	// the start kit: any of its fields declares it, start_kit_remove takes the section out
	EditorMod::SStartKit k = m.start_kit;
	if (NqUtil::ArgBool(raw, "start_kit_remove", false))
		k = EditorMod::SStartKit();
	// the tab's Game's Base Kit: the linked game's own rows and money, the fields below still apply on top
	if (NqUtil::ArgBool(raw, "start_kit_base", false))
	{
		EditorMod::SStartKit base;
		if (!GameBaseKit(base))
		{
			NqUtil::JsonError(out, "start_kit_base: the linked game has no plugins\\faction_loadouts.ltx [shared] kit to read");
			return;
		}
		base.perks = k.perks;
		k = base;
	}
	xr_string kit_text;
	if (NqUtil::ArgString(raw, "start_kit", kit_text))
	{
		// "device_torch, wpn_knife, bandage=2": the whole list, like the other list fields
		k.items.clear();
		xr_vector<xr_string> rows;
		NqUtil::Split(kit_text.c_str(), ',', rows);
		for (const xr_string& raw_row : rows)
		{
			const xr_string row = NqUtil::Trim(raw_row);
			if (row.empty()) continue;
			const size_t eq = row.find('=');
			EditorMod::SKitItem it;
			it.section = NqUtil::Trim(eq == xr_string::npos ? row : row.substr(0, eq));
			it.count = eq == xr_string::npos ? 1 : atoi(row.substr(eq + 1).c_str());
			if (it.section.empty() || it.count < 1)
			{
				NqUtil::JsonError(out, "start_kit: '" + row + "' needs an item section and a count of at least 1 (section or section=N)");
				return;
			}
			k.items.push_back(it);
		}
		k.declared = true;
	}
	// present or not: both defaults agree only when the field is there
	const int money_lo = NqUtil::ArgInt(raw, "start_money", INT_MIN), money_hi = NqUtil::ArgInt(raw, "start_money", INT_MAX);
	if (money_lo == money_hi)
	{
		k.money = money_lo < 0 ? -1 : money_lo;
		k.declared = true;
	}
	const int perks = ArgTri(raw, "start_perks");
	if (perks >= 0)
	{
		k.perks = perks == 1;
		k.declared = true;
	}
	m.start_kit = k;

	m.world = w;
	if (!EditorMod::Save(EditorProject::Root(), m))		{ NqUtil::JsonError(out, "can't write mod.ltx"); return; }
	WorldAnswer(m, out);
}

void EditorWorld::McpTransitions(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active())						{ NqUtil::JsonError(out, "no active project"); return; }
	if (!Available())
	{
		NqUtil::JsonError(out, xr_string("the linked game's transitions are not available: ") + Error());
		return;
	}
	SManifest m;
	EditorMod::Load(EditorProject::Root(), m);
	xr_string level;
	NqUtil::ArgString(raw, "level", level);

	out = "{\"ok\":true,\"levels\":[";
	bool first = true;
	for (const SLevel& l : Levels())
	{
		u32 n = 0;
		for (const STransition& t : Transitions()) if (t.level == l.name) ++n;
		if (!first) out += ",";
		first = false;
		out += "{\"name\":";		NqUtil::JsonString(out, l.name);
		out += ",\"caption\":";		NqUtil::JsonString(out, l.caption);
		out += NqUtil::Format(",\"transitions\":%u}", n);
	}
	out += "],\"transitions\":[";
	first = true;
	for (const STransition& t : Transitions())
	{
		if (!level.empty() && t.level != level) continue;
		if (!first) out += ",";
		first = false;
		const LPCSTR by = ClosedBy(m.world, t);
		out += "{\"name\":";		NqUtil::JsonString(out, t.name);
		out += ",\"level\":";		NqUtil::JsonString(out, t.level);
		out += ",\"to\":";			NqUtil::JsonString(out, t.to_level);
		out += ",\"to_caption\":";	NqUtil::JsonString(out, Caption(t.to_level.c_str()));
		out += ",\"closed\":";		NqUtil::JsonBool(out, by[0] != 0);
		out += ",\"closed_by\":";	NqUtil::JsonString(out, by);
		out += NqUtil::Format(",\"position\":[%.2f,%.2f,%.2f]}", t.position.x, t.position.y, t.position.z);
	}
	out += "]";
	// the game's own patrol paths, by a part of the name: routes an order may walk,
	// and names a module path must not take
	xr_string filter;
	if (NqUtil::ArgString(raw, "patrols", filter))
	{
		const int limit = NqUtil::ArgInt(raw, "limit", 200);
		out += ",\"patrols\":[";
		int n = 0;
		for (const xr_string& name : GamePatrols())
		{
			if (!filter.empty() && name.find(filter) == xr_string::npos) continue;
			if (n >= limit) break;
			if (n++) out += ",";
			NqUtil::JsonString(out, name);
		}
		out += "]";
		out += NqUtil::Format(",\"patrols_total\":%u", (u32)GamePatrols().size());
	}
	out += "}";
}
