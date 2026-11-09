#pragma once

#include "EditorModManifest.h"

//------------------------------------------------------------------------------
// The mode's world - what a module's mode keeps of the original game.
//
// A story mod on one map wants that map to itself: no stalkers of the game walking
// through, no squad leader handing out jobs, the way out closed until the story
// opens it, and the player starting where the author put them. The game does all of
// that from four things the module ships (game repo: docs/dead-air/MODDING.md,
// "World rules"):
//
//   mod.ltx [world]      what the mode keeps (five rules, each one a checkbox), where
//                        it applies, the playable area, the transitions that start
//                        closed, and the rules the player may change on the new-game
//                        screen                                  - this unit edits it
//   mod.ltx [start_kit]  what a new game begins with: the items, the money and whether
//                        the perks of the new-game screen stay; it replaces the screen's
//                        loadout                                 - this unit edits it
//   Player Start         a point of the spawn tool; the scene bake writes it into the
//                        spawn layer as `op = start`             - EditorModScene
//   way paths            the WayPoint tool's paths, shipped as [path:<name>] and
//                        registered by the engine as patrol paths - EditorModScene
//   NPC orders           a placed stalker's Order properties, shipped as [xms_order]
//                        in its custom data                       - CSpawnPoint
//
// Quests change the world while the game runs (world.* and order.* kinds of the
// catalog); this unit is what holds at the start.
//
// The linked game is read, never guessed: its levels and every level changer come
// out of its all.spawn (the same file the engine loads), so a transition is picked
// by the name the engine knows it by, under the level it stands on.
//------------------------------------------------------------------------------

namespace EditorWorld
{
	// a level of the linked game
	struct SLevel
	{
		xr_string	name;		// l03_agroprom
		xr_string	caption;	// the name the player reads (UTF-8), may be empty
	};

	// a level changer of the linked game
	struct STransition
	{
		xr_string	name;		// the object name - what [world] locked and world.transition name
		xr_string	level;		// where it stands
		xr_string	to_level;	// where it leads
		Fvector		position;
	};

	// a Player Start of the project: baked into a spawn layer, or placed in the open
	// scene and not baked yet
	struct SStart
	{
		xr_string	level;
		xr_string	name;		// the op (file) or the object (scene)
		xr_string	source;		// project-relative spawn file, or "scene"
		Fvector		position;
		Fvector		direction;
	};

	// -- the linked game ------------------------------------------------------
	// Read once per game link from its all.spawn. False when no game is linked or
	// the file could not be read - the lists are empty then and `Error` says why.
	ECORE_API bool							Available		();
	ECORE_API LPCSTR						Error			();
	ECORE_API const xr_vector<SLevel>&		Levels			();
	ECORE_API const xr_vector<STransition>&	Transitions		();
	ECORE_API bool							LevelExists		(LPCSTR name);
	ECORE_API const STransition*			FindTransition	(LPCSTR name);
	// the game's own patrol paths: a module path of the same name loses to them
	ECORE_API bool							GamePatrolExists(LPCSTR name);
	ECORE_API const xr_vector<xr_string>&	GamePatrols		();		// sorted
	// the player-facing name of a level, the id when it has none
	ECORE_API xr_string						Caption			(LPCSTR level);
	ECORE_API void							Invalidate		();

	// -- the project ----------------------------------------------------------
	// Every Player Start: the spawn layer's `op = start` entries, with the open
	// scene's own starts standing in for its level's file (the bake mirrors the
	// scene into that file, so the scene is what the next build ships).
	ECORE_API void		Starts			(LPCSTR project_root, xr_vector<SStart>& out);

	// Coded problems, the way quests and characters report them: E2nn stops a build,
	// W2nn is a warning.
	//   E201 more than one Player Start          E202 more than three player choices
	//   W203 a level the linked game lacks       W204 a transition the linked game lacks
	//   W205 the player starts outside the area  W206 an area without a Player Start
	//   E207 a start kit row with no item or a count below 1, or an item listed twice
	//   W208 a start kit item the linked game lacks
	ECORE_API void		Validate		(LPCSTR project_root, const EditorMod::SManifest& m,
										 xr_vector<xr_string>& problems);
	// the rows of the new-game screen a mode can offer
	static const u32	kMaxPlayerChoices = 3;

	// {"rules":{...},"player_choice":[...],"levels":[...],"area":[...],...}
	ECORE_API void		JsonWorld		(const EditorMod::SWorld& w, xr_string& out);
	// {"declared":true,"money":1,"perks":true,"items":[{"section":"wpn_knife","count":1},...]};
	// money is null while the kit leaves it to the new-game screen
	ECORE_API void		JsonStartKit	(const EditorMod::SStartKit& k, xr_string& out);
	// The game's own base kit - [shared] of the linked game's plugins\faction_loadouts.ltx,
	// what every faction gets before the extra kits: the rows and the money. False when
	// the game has no such file.
	ECORE_API bool		GameBaseKit		(EditorMod::SStartKit& out);

	// -- the open scene (the level editor lives a layer above) ----------------
	// Its level and the Player Starts placed in it.
	typedef void (*TSceneStarts)(xr_string& level, xr_vector<SStart>& starts);
	ECORE_API void		SetSceneStarts	(TSceneStarts fn);
	// Places a Player Start in the open scene in front of the camera and selects it.
	typedef bool (*TPlaceStart)(xr_string& err);
	ECORE_API void		SetPlaceStart	(TPlaceStart fn);
	// Selects a scene object by name and frames it.
	typedef bool (*TFocusObject)(LPCSTR name);
	ECORE_API void		SetFocusObject	(TFocusObject fn);

	// The levels of a list as chips, then Add Level...; true when the list changed.
	ECORE_API bool		LevelChips		(LPCSTR id, xr_vector<xr_string>& v);

	// -- UI (Mod > World Rules: a tab of its own) ---------------------------------
	ECORE_API void		DrawUI			();
	ECORE_API void		Show			();
	ECORE_API bool		IsOpen			();

	// -- MCP ----------------------------------------------------------------------
	// world_view: {"drawer":"open|close","level":"<name>","filter":"<text>","open":true} sets what the
	// World Rules tab shows and opens it; without an argument it only reports
	ECORE_API void		McpView			(LPCSTR raw, xr_string& out);
	ECORE_API void		McpGet			(LPCSTR raw, xr_string& out);		// world_get
	ECORE_API void		McpSet			(LPCSTR raw, xr_string& out);		// world_set
	ECORE_API void		McpTransitions	(LPCSTR raw, xr_string& out);		// world_transitions
}
