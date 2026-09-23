#pragma once

#include "Nq/NqAsset.h"		// SNqValue - texts keep the same shape quests use

//------------------------------------------------------------------------------
// Characters — the mod's own NPCs.
//
// A story mod needs stalkers of its own: a name the player reads, a model, a
// starting kit. X-Ray has one way to do that — a `specific_character` profile
// in `gameplay\character_desc_*.xml`, a `character` entry in a profiles xml,
// and both files listed in `[profiles]` of system.ltx — and the last part used
// to mean overwriting a file of the game, which is exactly what a module must
// never do.
//
// It does not have to. XMS composes configs: `+files` and
// `+specific_characters_files` in a `.ltxp` patch APPEND to those lists
// (xms_ltx.cpp), and an ltx overlay under `gamedata\configs\xms\` brings whole
// new sections, inheritance included. So a module ships its own two xml files,
// its own sections and a three-line patch, and clobbers nothing.
//
// That plumbing is what this unit writes. The author edits ONE file per
// character:
//
//   return {
//     npc       = 1,
//     id        = "courier",
//     name      = "Курьер",              -- or { rus = "…", eng = "…" }
//     community = "stalker",
//     visual    = "actors\\stalker_neutral\\stalker_neutral_1",
//     rank      = 300,
//     money     = { min = 500, max = 1500 },
//     supplies  = { { section = "wpn_ak74u", count = 1 },
//                   { section = "ammo_5.45x39_fmj", count = 3 } },
//   }
//
// and the build turns every `.npcasset` of the project into:
//
//   gamedata\configs\gameplay\character_desc_<module>.xml   the characters
//   gamedata\configs\gameplay\npc_profile_<module>.xml      their profiles
//   gamedata\configs\text\<lang>\<module>_npc.xml           names and bios
//   gamedata\configs\xms\<module>_npc.ltx                   spawn + squad sections
//   patch\<module>_characters.ltxp                          the three list lines
//
// The spawn section is why a character is usable everywhere at once: it carries
// `$spawn`, so the character appears in the editor's spawn roster and can be
// placed in a level by hand, AND a quest can spawn it with `spawn.object`.
// The one-man squad section makes `spawn.squad` work on it too.
//
// Generated files are owned by the editor (EditorMod::GetOwnedFiles): the
// content browser hides them, and a character that is deleted takes its
// generated half with it instead of haunting the module.
//------------------------------------------------------------------------------

namespace EditorCharacters
{
	struct SSupply
	{
		xr_string	section;
		// how many; 0 means "drop what the base gave me", which is the only way
		// to take one item out of an inherited kit
		int			count;

					SSupply	() : count(1) {}
	};

	struct SCharacter
	{
		xr_string	file;			// absolute path of the .npcasset (empty for a new one)
		xr_string	id;				// [a-z0-9_], unique in the module
		// Another character of this project to start from: every field this one
		// does not set comes from there, and the kits are merged. A chain is
		// allowed, a cycle is not. This is what stops the tenth guard from being
		// the same twenty lines again.
		xr_string	base;
		// A template, never a character in the game: no profile, no spawn
		// section, no string table. It exists to be inherited from, so it needs
		// no name and no model of its own.
		bool		abstract_;

		SNqValue	name;			// string, or a table keyed by language
		SNqValue	bio;

		xr_string	community;		// stalker, bandit, dolg, ... (from the linked game)
		xr_string	visual;			// meshes-relative, extension optional
		xr_string	icon;			// character icon in the dialog/PDA
		xr_string	start_dialog;
		xr_string	spec_rank;		// novice | experienced | veteran | master

		int			rank;
		int			reputation;
		bool		has_rank;
		bool		has_reputation;

		bool		has_money;
		int			money_min;
		int			money_max;
		bool		money_infinite;

		xr_vector<SSupply>	supplies;

		bool		squad;			// emit the one-man squad section (default true)
		bool		has_squad;		// set here, or inherited from the base

					SCharacter	();
		void		Reset		();
	};

	// -- the file ------------------------------------------------------------
	ECORE_API bool		Parse		(LPCSTR text, u32 len, LPCSTR chunk_name, SCharacter& c, xr_string& err);
	ECORE_API bool		Load		(LPCSTR path, SCharacter& c, xr_string& err);
	// canonical text: fixed key order, fixed layout, write(read(x)) idempotent
	ECORE_API xr_string	Write		(const SCharacter& c);
	ECORE_API bool		Save		(LPCSTR path, const SCharacter& c, xr_string& err);
	// a starting point for a new character, named after the file
	ECORE_API xr_string	Template	(LPCSTR id);

	// -- the project ---------------------------------------------------------
	ECORE_API void		List		(LPCSTR project_root, xr_vector<xr_string>& files);
	// every character of the project, sorted by id; a file that does not parse
	// becomes a problem line instead of stopping the walk
	ECORE_API void		LoadAll		(LPCSTR project_root, xr_vector<SCharacter>& out,
									 xr_vector<xr_string>& problems);
	// "<module>_<id>" - the id the game knows the character by, in the profile
	// list, the spawn section and the squad section alike
	ECORE_API xr_string	GameId		(LPCSTR module_id, LPCSTR char_id);

	// Flattens every `base` chain in place: each character comes back with the
	// fields and the kit it really has. A missing base or a cycle is reported
	// instead of looped over. Abstract characters stay in the list - the editor
	// still shows them, the BUILD is what leaves them out.
	ECORE_API void		Resolve		(xr_vector<SCharacter>& all, xr_vector<xr_string>& problems);
	// Everything, resolved, in one call - what the build, the validator and the
	// MCP answers all want.
	ECORE_API void		LoadResolved(LPCSTR project_root, xr_vector<SCharacter>& out,
									 xr_vector<xr_string>& problems);

	// -- who the project has -------------------------------------------------
	// What a quest names when it spawns a character (spawn.npc) and what the
	// pickers offer. CACHED: a graph is re-validated on every edit and must not
	// re-read every .npcasset each time - the list is rebuilt after Invalidate
	// (a character was saved or deleted, the browser refreshed) or a project
	// switch, and not otherwise.
	struct SBrief
	{
		xr_string	id;
		xr_string	name;		// the rus name, for the picker's caption
		xr_string	file;		// project-relative
		bool		abstract_;	// a template: never becomes an NPC, so it cannot be spawned
		SBrief() : abstract_(false) {}
	};
	ECORE_API const xr_vector<SBrief>&	Briefs		();
	ECORE_API void						Invalidate	();

	// -- validation ----------------------------------------------------------
	// Coded problems, the way quests report them: E<nnn> is fatal for a build,
	// W<nnn> is a warning. `linked_game` enables the checks that need the game
	// (does the visual exist, is the community real).
	ECORE_API void		Validate	(const SCharacter& c, bool linked_game,
									 xr_vector<xr_string>& problems);
	ECORE_API void		ValidateAll	(LPCSTR project_root, bool linked_game,
									 xr_vector<xr_string>& problems);

	// -- the build -----------------------------------------------------------
	// Writes (or removes, when the project has no characters) the generated
	// half into the project. Called by the export before anything is copied.
	ECORE_API bool		Generate	(LPCSTR project_root, LPCSTR module_id,
									 int& written, xr_string& err);
	ECORE_API void		GeneratedFiles(LPCSTR module_id, xr_vector<xr_string>& rel);

	// -- MCP -----------------------------------------------------------------
	ECORE_API void		McpList		(LPCSTR raw, xr_string& out);
	ECORE_API void		McpGet		(LPCSTR raw, xr_string& out);
	ECORE_API void		McpWrite	(LPCSTR raw, xr_string& out);
	ECORE_API void		McpDelete	(LPCSTR raw, xr_string& out);

	// -- UI ------------------------------------------------------------------
	// The model viewer lives a layer above this dll, so the editor hands over the
	// function that opens it on a model (meshes-relative, extension optional) - the
	// window's Preview button next to the model.
	typedef void (*TModelPreview)(LPCSTR visual);
	ECORE_API void		SetModelPreview	(TModelPreview fn);
	ECORE_API void		DrawUI		();
	ECORE_API void		Show		();
	ECORE_API void		Open		(LPCSTR path);		// from the content browser
	ECORE_API bool		IsOpen		();
}
