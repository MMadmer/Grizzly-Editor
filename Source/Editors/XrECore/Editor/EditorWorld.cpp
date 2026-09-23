#include "stdafx.h"
#include "EditorWorld.h"
#include "EditorProject.h"
#include "EditorGameContent.h"
#include "EditorGameConfigs.h"
#include "EditorGameModes.h"
#include "XFinedMCP.h"
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
	// the window
	//--------------------------------------------------------------------------
	bool			s_Open			= false;
	SWorld			s_World;				// what mod.ltx says, reloaded when it changes
	SManifest		s_Manifest;
	bool			s_HaveManifest	= false;
	u32				s_WorldGen		= u32(-1);
	xr_string		s_WorldRoot;
	xr_string		s_Status;
	xr_string		s_TransLevel;			// the level whose transitions are listed
	char			s_Search[64]	= {};
	// what the window shows about the project, refreshed a second apart: the spawn
	// layer is files, and a window must not walk them every frame
	xr_vector<EditorWorld::SStart>	s_ShownStarts;
	xr_vector<xr_string>			s_ShownProblems;
	xr_string						s_SceneLevel;
	u32								s_CheckedAt		= 0;

	struct SRuleText { LPCSTR caption; LPCSTR hint; };
	const SRuleText kRules[EditorMod::wrCount] =
	{
		{ "Original characters",	"Traders, story characters and every squad the game placed by name (a story id).\nOff: they are not in the world at all." },
		{ "Random stalkers",		"The simulation's stalker squads: the ones placed at the start and the ones smart terrains bring back." },
		{ "Mutants",				"The simulation's monster squads, placed at the start and respawned." },
		{ "Jobs from NPCs",			"\"Got any work?\" - the jobs squad leaders hand out: fetch, kill, escort.\nOff: the offer is not in their dialogs." },
		{ "Story quests",			"Tasks of the named characters and the storylines.\nOff: their offers are not in the dialogs." },
	};

	bool LoadWorld(SWorld& w, SManifest& m)
	{
		if (!EditorProject::Active() || !EditorMod::Load(EditorProject::Root(), m)) return false;
		w = m.world;
		return true;
	}

	// every edit is saved at once, like Exclude from Build: the manifest is re-read
	// first, so the Manifest window's own edits are never written over
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
		s_Status = SaveWorld(s_World, err) ? xr_string() : "Save failed: " + err;
	}

	// a level picker: the linked game's levels, minus the ones listed already
	bool PickLevel(LPCSTR popup, const xr_vector<xr_string>& taken, xr_string& picked)
	{
		bool done = false;
		if (!ImGui::BeginPopup(popup)) return false;
		if (ImGui::IsWindowAppearing()) { s_Search[0] = 0; ImGui::SetKeyboardFocusHere(); }
		ImGui::SetNextItemWidth(-1.f);
		ImGui::InputTextWithHint("##search", "search", s_Search, sizeof(s_Search));
		const xr_vector<EditorWorld::SLevel>& levels = EditorWorld::Levels();
		if (levels.empty())
			ImGui::TextDisabled("link a game to pick its levels");
		const float em = ImGui::GetFontSize();
		ImGui::BeginChild("##levels", ImVec2(em * 24.f, em * 18.f));
		for (const EditorWorld::SLevel& l : levels)
		{
			if (Contains(taken, l.name)) continue;
			xr_string label = l.name;
			if (!l.caption.empty()) label += "  -  " + l.caption;
			if (s_Search[0] && !strstr(label.c_str(), s_Search)) continue;
			if (ImGui::Selectable(label.c_str())) { picked = l.name; done = true; ImGui::CloseCurrentPopup(); }
		}
		ImGui::EndChild();
		ImGui::EndPopup();
		return done;
	}

	// chips with a remove button, then "+ Add"; true when the list changed
	bool LevelList(LPCSTR id, xr_vector<xr_string>& v)
	{
		bool changed = false;
		ImGui::PushID(id);
		for (u32 i = 0; i < v.size(); ++i)
		{
			ImGui::PushID((int)i);
			const bool unknown = EditorWorld::Available() && !EditorWorld::LevelExists(v[i].c_str());
			if (unknown) ImGui::PushStyleColor(ImGuiCol_Text, ImVec4(1.f, 0.72f, 0.3f, 1.f));
			ImGui::TextUnformatted(v[i].c_str());
			if (unknown) ImGui::PopStyleColor();
			if (ImGui::IsItemHovered())
				ImGui::SetTooltip("%s", unknown ? "the linked game has no level of this name" : EditorWorld::Caption(v[i].c_str()).c_str());
			ImGui::SameLine();
			if (ImGui::SmallButton("x")) { v.erase(v.begin() + i); changed = true; ImGui::PopID(); break; }
			ImGui::SameLine();
			ImGui::PopID();
		}
		if (ImGui::SmallButton("+ Add level")) ImGui::OpenPopup("pick_level");
		xr_string picked;
		if (PickLevel("pick_level", v, picked)) { v.push_back(picked); changed = true; }
		ImGui::PopID();
		return changed;
	}

	// "Every level" / "Only these levels" over one list; an emptied list means everywhere
	bool Scope(LPCSTR id, LPCSTR everywhere, LPCSTR only, xr_vector<xr_string>& v, bool& limited)
	{
		bool changed = false;
		ImGui::PushID(id);
		if (ImGui::RadioButton(everywhere, !limited))
		{
			if (limited) { limited = false; changed = !v.empty(); v.clear(); }
		}
		ImGui::SameLine();
		if (ImGui::RadioButton(only, limited)) limited = true;
		if (limited)
		{
			ImGui::Indent();
			changed |= LevelList("levels", v);
			ImGui::Unindent();
		}
		ImGui::PopID();
		return changed;
	}

	xr_string ModeText(const SManifest& m)
	{
		if (!m.provides_modes.empty())
			return "its own mode '" + (m.provides_modes[0].title.empty() ? m.provides_modes[0].id : m.provides_modes[0].title) + "'";
		if (!m.mode.empty())
		{
			for (const EditorGameModes::SMode& mode : EditorGameModes::All())
				if (mode.id == m.mode) return "the mode '" + mode.caption + "'";
			return "the mode '" + m.mode + "'";
		}
		if (0 == _stricmp(m.target.c_str(), "default")) return "every new game (the module targets the plain game)";
		return "no mode yet - pick one in Mod > Edit Manifest";
	}

	bool s_AreaLimited = false, s_LevelsLimited = false;

	void DrawRules()
	{
		const float em = ImGui::GetFontSize();
		if (!ImGui::CollapsingHeader("Original game", ImGuiTreeNodeFlags_DefaultOpen)) return;
		if (ImGui::BeginTable("##rules", 3, ImGuiTableFlags_SizingFixedFit | ImGuiTableFlags_RowBg))
		{
			ImGui::TableSetupColumn("The game's own", ImGuiTableColumnFlags_WidthStretch);
			ImGui::TableSetupColumn("Keep");
			ImGui::TableSetupColumn("Player decides");
			ImGui::TableHeadersRow();
			const u32 choices = ChoiceCount(s_World);
			for (u32 i = 0; i < EditorMod::wrCount; ++i)
			{
				ImGui::PushID((int)i);
				ImGui::TableNextRow();
				ImGui::TableNextColumn();
				ImGui::TextUnformatted(kRules[i].caption);
				if (ImGui::IsItemHovered()) ImGui::SetTooltip("%s\n\nmod.ltx: %s", kRules[i].hint, EditorMod::WorldRuleKey(i));
				ImGui::TableNextColumn();
				bool keep = s_World.keep[i];
				if (ImGui::Checkbox("##keep", &keep)) { s_World.keep[i] = keep; Commit(); }
				ImGui::TableNextColumn();
				bool choice = s_World.player_choice[i];
				ImGui::BeginDisabled(!choice && choices >= EditorWorld::kMaxPlayerChoices);
				if (ImGui::Checkbox("##choice", &choice)) { s_World.player_choice[i] = choice; Commit(); }
				ImGui::EndDisabled();
				if (ImGui::IsItemHovered(ImGuiHoveredFlags_AllowWhenDisabled))
					ImGui::SetTooltip("A checkbox on the new-game screen, set to what Keep says.\n"
									  "The screen has room for %u.", EditorWorld::kMaxPlayerChoices);
				ImGui::PopID();
			}
			ImGui::EndTable();
		}
		if (ImGui::Button("Keep everything"))
		{
			for (u32 i = 0; i < EditorMod::wrCount; ++i) s_World.keep[i] = true;
			Commit();
		}
		ImGui::SameLine();
		if (ImGui::Button("Remove everything"))
		{
			for (u32 i = 0; i < EditorMod::wrCount; ++i) s_World.keep[i] = false;
			Commit();
		}
		if (ImGui::IsItemHovered())
			ImGui::SetTooltip("An empty world: no NPC, no monster and no task of the original game.\nYour own characters and quests stay.");
		ImGui::Spacing();
		ImGui::TextUnformatted("Applies on");
		ImGui::SameLine(em * 7.f);
		if (Scope("levels", "Every level", "These levels", s_World.levels, s_LevelsLimited)) Commit();
	}

	void DrawArea()
	{
		if (!ImGui::CollapsingHeader("Playable area", ImGuiTreeNodeFlags_DefaultOpen)) return;
		if (Scope("area", "Anywhere", "Only these levels", s_World.area, s_AreaLimited)) Commit();
		ImGui::TextDisabled("A transition out of the area stays closed. Quests open it (world.transition) or grow the area (world.area).");
	}

	void DrawTransitions(const xr_string& scene_level)
	{
		if (!ImGui::CollapsingHeader("Transitions", ImGuiTreeNodeFlags_DefaultOpen)) return;
		const xr_vector<EditorWorld::STransition>& all = EditorWorld::Transitions();
		if (!EditorWorld::Available())
		{
			ImGui::TextDisabled("%s", EditorWorld::Error()[0] ? EditorWorld::Error() : "link a game to list its transitions");
			return;
		}
		// the scene's level first, then the first of the area, then the first with any
		if (s_TransLevel.empty())
		{
			if (!scene_level.empty())			s_TransLevel = scene_level;
			else if (!s_World.area.empty())		s_TransLevel = s_World.area[0];
			else if (!all.empty())				s_TransLevel = all[0].level;
		}
		const float em = ImGui::GetFontSize();
		ImGui::TextUnformatted("Level");
		ImGui::SameLine(em * 7.f);
		ImGui::SetNextItemWidth(em * 22.f);
		xr_string preview = s_TransLevel + (EditorWorld::Caption(s_TransLevel.c_str()) != s_TransLevel ? "  -  " + EditorWorld::Caption(s_TransLevel.c_str()) : "");
		if (ImGui::BeginCombo("##trans_level", preview.c_str()))
		{
			for (const EditorWorld::SLevel& l : EditorWorld::Levels())
			{
				u32 n = 0;
				for (const EditorWorld::STransition& t : all) if (t.level == l.name) ++n;
				if (!n) continue;
				xr_string label = NqUtil::Format("%s  -  %s  (%u)", l.name.c_str(), l.caption.empty() ? "" : l.caption.c_str(), n);
				if (ImGui::Selectable(label.c_str(), l.name == s_TransLevel)) s_TransLevel = l.name;
			}
			ImGui::EndCombo();
		}
		bool level_locked = Contains(s_World.locked_levels, s_TransLevel);
		if (ImGui::Checkbox("Close every transition on this level", &level_locked))
		{
			Toggle(s_World.locked_levels, s_TransLevel, level_locked);
			Commit();
		}
		if (ImGui::BeginTable("##trans", 3, ImGuiTableFlags_SizingFixedFit | ImGuiTableFlags_RowBg | ImGuiTableFlags_ScrollY,
			ImVec2(0.f, em * 12.f)))
		{
			ImGui::TableSetupScrollFreeze(0, 1);
			ImGui::TableSetupColumn("Transition", ImGuiTableColumnFlags_WidthStretch);
			ImGui::TableSetupColumn("Leads to", ImGuiTableColumnFlags_WidthStretch);
			ImGui::TableSetupColumn("Closed");
			ImGui::TableHeadersRow();
			for (const EditorWorld::STransition& t : all)
			{
				if (t.level != s_TransLevel) continue;
				ImGui::PushID(t.name.c_str());
				ImGui::TableNextRow();
				ImGui::TableNextColumn();
				ImGui::TextUnformatted(t.name.c_str());
				if (ImGui::IsItemHovered()) ImGui::SetTooltip("at %.1f, %.1f, %.1f", t.position.x, t.position.y, t.position.z);
				ImGui::TableNextColumn();
				ImGui::TextUnformatted(EditorWorld::Caption(t.to_level.c_str()).c_str());
				if (ImGui::IsItemHovered()) ImGui::SetTooltip("%s", t.to_level.c_str());
				ImGui::TableNextColumn();
				const LPCSTR by = ClosedBy(s_World, t);
				bool closed = by[0] != 0;
				const bool own = 0 == xr_strcmp(by, "locked") || !by[0];
				ImGui::BeginDisabled(!own);
				if (ImGui::Checkbox("##closed", &closed))
				{
					Toggle(s_World.locked, t.name, closed);
					Commit();
				}
				ImGui::EndDisabled();
				if (!own && ImGui::IsItemHovered(ImGuiHoveredFlags_AllowWhenDisabled))
					ImGui::SetTooltip("%s", 0 == xr_strcmp(by, "level") ? "closed with the whole level" : "closed: it leads out of the playable area");
				ImGui::PopID();
			}
			ImGui::EndTable();
		}
		// names the game does not have: kept, shown, removable
		for (u32 i = 0; i < s_World.locked.size(); ++i)
		{
			if (EditorWorld::FindTransition(s_World.locked[i].c_str())) continue;
			ImGui::PushID((int)i);
			ImGui::TextColored(ImVec4(1.f, 0.72f, 0.3f, 1.f), "closed, but not in the linked game: %s", s_World.locked[i].c_str());
			ImGui::SameLine();
			if (ImGui::SmallButton("remove")) { s_World.locked.erase(s_World.locked.begin() + i); Commit(); ImGui::PopID(); break; }
			ImGui::PopID();
		}
	}

	void DrawStart(const xr_string& scene_level)
	{
		if (!ImGui::CollapsingHeader("Player start", ImGuiTreeNodeFlags_DefaultOpen)) return;
		const xr_vector<EditorWorld::SStart>& starts = s_ShownStarts;
		if (starts.empty())
			ImGui::TextDisabled("None: a new game starts where the faction's start puts it.");
		for (u32 i = 0; i < starts.size(); ++i)
		{
			const EditorWorld::SStart& s = starts[i];
			ImGui::PushID((int)i);
			ImGui::Text("%s  (%s)  at %.1f, %.1f, %.1f", EditorWorld::Caption(s.level.c_str()).c_str(), s.level.c_str(),
				s.position.x, s.position.y, s.position.z);
			if (ImGui::IsItemHovered()) ImGui::SetTooltip("%s: %s", s.source.c_str(), s.name.c_str());
			if (s.source == "scene" && s_FocusObject)
			{
				ImGui::SameLine();
				if (ImGui::SmallButton("Show")) s_FocusObject(s.name.c_str());
			}
			ImGui::PopID();
		}
		if (starts.size() > 1)
			ImGui::TextColored(ImVec4(1.f, 0.4f, 0.4f, 1.f), "A module has one Player Start - the build refuses %u.", (u32)starts.size());
		ImGui::BeginDisabled(scene_level.empty() || !s_PlaceStart);
		if (ImGui::Button("Place in the scene"))
		{
			xr_string err;
			if (!s_PlaceStart(err)) s_Status = "Place failed: " + err;
			s_CheckedAt = 0;
		}
		ImGui::EndDisabled();
		if (ImGui::IsItemHovered(ImGuiHoveredFlags_AllowWhenDisabled))
			ImGui::SetTooltip(scene_level.empty() ? "open the level's scene first" :
				"in front of the camera; the arrow is where the player faces.\nThe Spawn tool has it too: Player Start.");
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
void EditorWorld::Show()	{ s_Open = true; }
bool EditorWorld::IsOpen()	{ return s_Open; }

void EditorWorld::DrawUI()
{
	if (!s_Open) return;
	const float em = ImGui::GetFontSize();
	ImGui::SetNextWindowSize(ImVec2(em * 40.f, em * 46.f), ImGuiCond_FirstUseEver);
	if (!ImGui::Begin("World", &s_Open))
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

	// mod.ltx is the one truth: re-read whenever anything saved it
	if (s_WorldGen != EditorMod::ManifestGeneration() || s_WorldRoot != EditorProject::Root())
	{
		const bool new_project = s_WorldRoot != EditorProject::Root();
		s_HaveManifest = EditorMod::Load(EditorProject::Root(), s_Manifest);
		s_World = s_Manifest.world;
		s_WorldGen = EditorMod::ManifestGeneration();
		s_WorldRoot = EditorProject::Root();
		s_AreaLimited |= !s_World.area.empty();
		s_LevelsLimited |= !s_World.levels.empty();
		if (new_project)
		{
			s_AreaLimited = !s_World.area.empty();
			s_LevelsLimited = !s_World.levels.empty();
			s_TransLevel.clear();
		}
		s_CheckedAt = 0;
	}
	if (!s_HaveManifest)
	{
		ImGui::TextDisabled("This project has no mod.ltx yet: Mod > Edit Manifest.");
		ImGui::End();
		return;
	}
	const u32 now = ::GetTickCount();
	if (!s_CheckedAt || now - s_CheckedAt > 1000)
	{
		s_CheckedAt = now ? now : 1;
		Starts(EditorProject::Root(), s_ShownStarts);
		s_ShownProblems.clear();
		Validate(EditorProject::Root(), s_Manifest, s_ShownProblems);
		s_SceneLevel.clear();
		if (s_SceneStarts)
		{
			xr_vector<SStart> unused;
			s_SceneStarts(s_SceneLevel, unused);
		}
	}
	const SManifest& m = s_Manifest;
	const xr_string& scene_level = s_SceneLevel;

	ImGui::TextWrapped("These rules shape %s.", ModeText(m).c_str());
	if (ImGui::IsItemHovered())
		ImGui::SetTooltip("A new game reads them once and keeps them in its saves.\nQuests change them while the game runs: the world.* kinds.");
	ImGui::SameLine();
	if (ImGui::SmallButton("Reload game data"))
	{
		Invalidate();
		NqPickers::Invalidate();
	}
	ImGui::Separator();

	ImGui::BeginChild("##world", ImVec2(0.f, -em * 5.f));
	DrawRules();
	DrawArea();
	DrawTransitions(scene_level);
	DrawStart(scene_level);
	ImGui::EndChild();

	// ---- problems ------------------------------------------------------------
	ImGui::Separator();
	const xr_vector<xr_string>& problems = s_ShownProblems;
	ImGui::BeginChild("##problems", ImVec2(0.f, 0.f));
	if (!s_Status.empty()) ImGui::TextColored(ImVec4(1.f, 0.4f, 0.4f, 1.f), "%s", s_Status.c_str());
	if (problems.empty() && s_Status.empty()) ImGui::TextDisabled("No problems.");
	for (const xr_string& p : problems)
		ImGui::TextColored(p[0] == 'E' ? ImVec4(1.f, 0.4f, 0.4f, 1.f) : ImVec4(1.f, 0.72f, 0.3f, 1.f), "%s", p.c_str());
	ImGui::EndChild();
	ImGui::End();
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
