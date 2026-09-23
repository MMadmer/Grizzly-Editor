#pragma once

// XMS (XFined Module System) mod-authoring support.
//
// Every project doubles as an XMS module package: a mod.ltx manifest at the
// project root plus the module dirs (gamedata\, patch\, spawn\, scripts\).
// This unit owns the manifest parse/write, the scaffold and both export
// paths; the LevelEditor menu and the MCP inspector are thin callers.
// Plain statics in a namespace, same style as EditorProject.

namespace EditorMod
{
	// a game mode this module declares ([provides_mode]: id + title string id)
	struct SProvidesMode
	{
		xr_string				id;
		xr_string				title;
	};

	// The five world rules, in the order the game's XMS::WorldRules keeps them. Each
	// says whether the ORIGINAL game's content of that kind stays in the module's mode.
	enum EWorldRule { wrOriginalNpcs = 0, wrRandomStalkers, wrMutants, wrNpcQuests, wrStoryQuests, wrCount };
	// manifest key of a rule ("original_npcs", ...); null for an unknown index
	ECORE_API LPCSTR	WorldRuleKey		(u32 rule);
	// rule index of a manifest key; wrCount when unknown
	ECORE_API u32		WorldRuleFromKey	(LPCSTR key);

	// mod.ltx [world] (game repo: docs/dead-air/MODDING.md, "World rules"): what of the
	// original game the module's mode keeps, which rules the player may change on the
	// new-game screen, where the player may go and which ways start closed. The engine
	// reads it; it takes effect on a new game.
	struct SWorld
	{
		bool					keep[wrCount] = { true, true, true, true, true };
		bool					player_choice[wrCount] = {};
		xr_vector<xr_string>	levels;			// where the rules apply; empty = everywhere
		xr_vector<xr_string>	area;			// the only levels the player may reach; empty = anywhere
		xr_vector<xr_string>	locked_levels;	// every transition on these levels starts closed
		xr_vector<xr_string>	locked;			// these transitions (level changer names) start closed
		xr_vector<xr_string>	extra;			// unknown [world] lines, kept as written
		// true when the world is the stock game's - the section is then not written
		bool					Empty() const;
	};

	// parsed mod.ltx; unknown keys/sections ([budget], [declares], engine_min,
	// ...) survive a load/save round-trip verbatim in the *_extra fields
	struct SManifest
	{
		xr_string				id;
		xr_string				name;
		xr_string				version;
		// Mods menu metadata (game repo: docs/dead-air/MOD_UPDATES.md, section 1).
		// name/author/description hold the decoded text - Save quotes and escapes.
		xr_string				author;
		xr_string				description;	// may hold real line breaks
		xr_string				website;		// AP-PRO or ModDB page, see ValidateWebsite
		xr_string				update_github;	// [update] github: owner/repo the game checks for releases
		xr_string				api;
		xr_string				mode;			// [module] mode: gate content by game mode, empty = all modes
		// [module] target: which of the three targeting choices the author made.
		// "default" = the ordinary game, deliberately. Empty means nothing was
		// chosen yet, and the module export refuses that - shipping a module
		// whose mode nobody decided on is how content lands in the wrong game.
		xr_string				target;
		xr_vector<xr_string>	requires_list;	// raw entries, e.g. "other.mod = >=1.0"
		xr_vector<xr_string>	after;			// module ids from [order]
		xr_vector<xr_string>	before;
		xr_vector<xr_string>	conflicts;		// raw entries
		xr_vector<SProvidesMode> provides_modes;	// declared game modes
		xr_vector<xr_string>	provides_extra;	// unknown [provides_mode] lines
		xr_vector<xr_string>	update_extra;	// unknown [update] lines
		xr_vector<xr_string>	module_extra;	// unknown [module] lines
		xr_vector<xr_string>	extra;			// unknown sections, headers included
		// [exclude]: what the build leaves out on the author's say - project paths
		// or patterns, one per line, kept as written (see ExcludeMatch)
		xr_vector<xr_string>	exclude;
		// [world]: the mode's world (EditorWorld)
		SWorld					world;
	};

	// id charset per XMS spec: [a-z0-9_.\-]+
	ECORE_API bool	ValidateId			(LPCSTR id);
	// mode ids follow the module id charset, but empty is allowed (= all modes)
	ECORE_API bool	ValidateModeId		(LPCSTR id);
	// strict form of MOD_UPDATES.md 1.2: 1-4 dot-separated numbers of at most
	// 9 digits and nothing else, 32 characters in all - a release has to be orderable
	ECORE_API bool	ValidateVersion		(LPCSTR version);
	// MOD_UPDATES.md 1.3: https, host exactly (www.)ap-pro.ru or (www.)moddb.com,
	// no user info or port, limited charset, 512 bytes at most; empty is valid
	ECORE_API bool	ValidateWebsite		(LPCSTR url);
	// MOD_UPDATES.md 1.4: owner/repo; empty is valid
	ECORE_API bool	ValidateGithubRepo	(LPCSTR repo);
	// reduces a pasted https://github.com/owner/repo[.git][/...] to owner/repo;
	// anything else comes back trimmed for ValidateGithubRepo to judge
	ECORE_API void	NormalizeGithubRepo	(LPCSTR in, xr_string& out);
	// project name -> module id: lowercased, invalid chars become '_'
	ECORE_API void	DeriveId			(LPCSTR project_name, char* dst, u32 dst_size);

	// creates missing mod.ltx / module dirs / patch readme; never overwrites
	ECORE_API void	EnsureScaffold		(LPCSTR project_root, LPCSTR project_name);

	// Project-relative paths of the files the EDITOR owns inside the project:
	// the manifest and everything export (re)generates. The content browser
	// hides them - they are required machinery, not content, and deleting one
	// by accident must not be possible - and the export's mode step deletes
	// the generated ones when the declaration they came from is gone.
	ECORE_API void	GetOwnedFiles		(LPCSTR project_root, xr_vector<xr_string>& rel);

	ECORE_API bool	Load				(LPCSTR project_root, SManifest& m);
	ECORE_API bool	Save				(LPCSTR project_root, const SManifest& m);
	// bumped by every Save: a window showing part of the manifest re-reads it when
	// this moves instead of asking the disk every frame
	ECORE_API u32	ManifestGeneration	();

	// ---- what the build leaves out -----------------------------------------
	// A project folder is the author's working place as well as the module: a git
	// repository, design notes, reference pictures. Tooling never ships (see
	// EditorProject::IsToolingEntry); the rest is the author's call, made in the
	// [exclude] section of mod.ltx. An entry there is a project-relative path - a
	// folder takes everything below it - or a pattern: with no slash it matches a
	// NAME at any depth (*.psd), with one a path from the root (docs/*.md). '*' and
	// '?' never cross a folder boundary; case and slash direction do not matter.
	// `rule` comes back with the entry that matched.
	ECORE_API bool	ExcludeMatch		(const xr_vector<xr_string>& rules, LPCSTR rel, xr_string* rule = 0);
	// A project's [exclude] list, read once and kept until mod.ltx is saved or
	// InvalidateExclusions is called: the content browser asks per tile, per frame.
	ECORE_API const xr_vector<xr_string>& Exclusions(LPCSTR project_root);
	ECORE_API void	InvalidateExclusions();
	// Is this project-relative entry out of the build: tooling at any depth, the
	// editor's scratch and the sources at the root, whatever [exclude] names. Every
	// walk that feeds a build asks it - the graphs, the characters, the export.
	ECORE_API bool	OutsideBuild		(LPCSTR project_root, LPCSTR rel);
	// Adds or removes one project-relative entry and saves mod.ltx. Removing fails,
	// naming the rule, when something else still excludes the entry.
	ECORE_API bool	SetExcluded			(LPCSTR project_root, LPCSTR rel, bool excluded, xr_string& err);
	// What the content browser says about an entry: it ships; its own [exclude] line
	// takes it out; another rule does (a folder above it, a pattern - `rule` names
	// it); or it never ships at all (the editor's scratch, the sources).
	enum EShipState { shipShipped = 0, shipExcluded, shipExcludedByRule, shipNever };
	ECORE_API EShipState ShipState		(LPCSTR project_root, LPCSTR rel, xr_string* rule = 0);

	// flat=false: mod.ltx + module dirs -> <target>\modules\<id>\
	// flat=true : gamedata content -> <target>\gamedata_<id>\ + compat report
	// The module export MIRRORS the project: files the target holds and the
	// project does not are deleted. When those exist and `confirmed` is false
	// it refuses instead, so the caller can ask first - the target may be
	// someone else's module that shares the id.
	// `warnings` (optional) comes back with the quest problems that did not stop the
	// build: a warning nobody is shown is a warning that does not exist, and the one
	// that matters most - a quest naming a restrictor that was never placed - reads as
	// a clean build otherwise.
	// `left_out` (optional) comes back with what the build passed by and why it
	// matters to know: every tooling entry, every [exclude]d one and every link,
	// project-relative, at the level the walk stopped.
	ECORE_API bool	Export				(LPCSTR project_root, LPCSTR target_root, bool flat,
										 int& files, xr_string& out_path, xr_string& err,
										 bool confirmed = false, xr_string* warnings = 0,
										 xr_vector<xr_string>* left_out = 0);

	// one asset of a release, as the descriptor lists it
	struct SPackageFile
	{
		xr_string				name;			// asset name: <id>-<version>[.partN].zip, <id>-<version>.files
		u64						size;
		xr_string				sha256;			// lowercase hex

								SPackageFile	() : size(0) {}
	};

	struct SPackageResult
	{
		xr_string				out_dir;		// folder that holds everything below
		xr_string				descriptor;		// full path of <id>.update.ltx
		SPackageFile			descriptor_file;	// the same file as a release asset
		SPackageFile			index;			// the file index, <id>-<version>.files
		xr_vector<SPackageFile>	packages;
		xr_string				requires_game;	// as written, empty = key omitted
		int						files;			// files in all packages together
		u64						unpacked;		// their size before compression
		u64						packed;			// all packages together

		// what the staging build passed by (Export's left_out), for the release report
		xr_vector<xr_string>	left_out;

								SPackageResult	() : files(0), unpacked(0), packed(0) {}
	};

	// A release for the game's module updater: <id>.update.ltx, the file index
	// <id>-<version>.files and package zip(s) holding the COMPLETE module under
	// modules/<id>/. The index says where inside a package every file's data
	// sits, so the game downloads only the files that differ. Names, descriptor
	// and index format and package layout are a frozen contract with the game -
	// the normative text is docs/dead-air/MOD_UPDATES.md (section 2) in the game repo.
	// The module is staged through Export, so everything a module build refuses
	// is refused here too. `out_dir` empty = <project>\releases\<version>;
	// `requires_game` empty = the key is left out. Files an earlier packaging of
	// the same id+version left in `out_dir` are replaced (deleted) without
	// asking - the UI asks first. Callers run RunPreBuildBake, like every build.
	ECORE_API bool	PackageRelease		(LPCSTR project_root, LPCSTR out_dir, LPCSTR requires_game,
										 SPackageResult& result, xr_string& err, xr_string* warnings = 0);
	// What an earlier packaging of this id + version left in the release folder -
	// the files the next one deletes. A dialog names them first.
	ECORE_API void	ListPackagedFiles	(LPCSTR project_root, LPCSTR out_dir, LPCSTR id, LPCSTR version,
										 xr_vector<xr_string>& names);
	// MAJOR.MINOR.PATCH from the version resource of <linked game>\xrEngine.exe
	ECORE_API bool	LinkedGameVersion	(xr_string& out);

	// modal windows (drawn from CLevelMain::OnDrawUI) + Mod menu triggers
	ECORE_API void	DrawUI				();
	ECORE_API void	RequestEditManifest	();
	ECORE_API void	RequestExportModule	();
	ECORE_API void	RequestExportFlat	();
	ECORE_API void	RequestPackageRelease();

	// the everyday build: no target picker, no options - the linked game IS
	// the target, so this is Export(flat=false) straight to <game>\modules\<id>
	ECORE_API void	RequestBuildIntoGame();
	ECORE_API bool	CanBuildIntoGame	();
	// "<game>\modules\<id>" for a menu tooltip; empty when it can't be built
	ECORE_API void	BuildTargetText		(char* dst, u32 size);

	// Scene work lives a layer above this dll, but a build must ship what the
	// author sees - so the editor registers a bake here and every build path
	// (menu, modal, MCP) runs it before Export. The note goes into the result.
	typedef void (*TPreBuildBake)(xr_string& note);
	ECORE_API void	SetPreBuildBake		(TPreBuildBake fn);
	ECORE_API void	RunPreBuildBake		(xr_string& note);

	// MCP command handlers: fill the full JSON response; raw = request line
	ECORE_API void	McpManifest			(xr_string& out);
	ECORE_API void	McpSetManifest		(LPCSTR raw, xr_string& out);
	ECORE_API void	McpExport			(LPCSTR raw, xr_string& out);
	// {"path":"docs","excluded":true|false} - the browser's Exclude from Build
	ECORE_API void	McpExclude			(LPCSTR raw, xr_string& out);
	ECORE_API void	McpPackage			(LPCSTR raw, xr_string& out);
}
