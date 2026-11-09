#pragma once

// XMS (Grizzly Module System) mod-authoring support.
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

	// one row of a start kit: an item section and how many of it (ammo: boxes)
	struct SKitItem
	{
		xr_string				section;
		int						count = 1;
	};

	// mod.ltx [start_kit] (game repo: docs/dead-air/MODDING.md, "The start kit"): what a
	// new game of the module's mode begins with. It REPLACES the new-game screen's loadout -
	// the faction's kit and the extra kits picked there, and the perks when `perks` is off.
	// The section being there is what declares a kit: an empty one starts the player with
	// nothing but the PDA.
	struct SStartKit
	{
		bool					declared = false;
		int						money = -1;		// -1 = not written: the new-game screen's money stays
		bool					perks = true;	// the perks picked on the new-game screen stay
		xr_vector<SKitItem>		items;			// in the order written
	};

	// mod.ltx [vars]: the module's global variables (NQ) - `name = default`, one value every
	// quest and behaviour graph of the module reads and writes (global.set, global.add, the
	// `global` condition, {global:name} in a text). The default is kept as written: true /
	// false is a bool, a number a number, anything else a string (quoted when the editor
	// writes it, so "1" stays text). Game half: xms_nq_load.parse_globals.
	struct SGlobalVar
	{
		xr_string	name;		// [A-Za-z0-9_]
		xr_string	value;		// as written, quotes included
	};

	// mod.ltx [commands] and [command.<name>] (game repo: docs/dead-air/MODDING.md, "Console
	// commands"): a console command the module adds. `name = <script>.<function>` in [commands];
	// the game calls the function of the project's gamedata\scripts with the text typed after the
	// name and prints what it returns. help, args and cheat live in the command's own section.
	struct SCommand
	{
		xr_string				name;			// a-z, 0-9 and _ (ValidCommandName)
		xr_string				function;		// <script>.<function>
		xr_string				help;			// decoded text or a string table id; Save quotes it when it has to
		xr_string				args;			// what it takes, e.g. "<night 1-7>"
		bool					cheat = false;	// only in Dead Air's debug mode (-dbg)
		bool					declared = true;	// false: only a [command.<name>] section names it
		xr_vector<xr_string>	extra;			// unknown lines of its section, kept as written
	};

	// parsed mod.ltx; unknown keys/sections ([budget], [declares], engine_min,
	// ...) survive a load/save round-trip verbatim in the *_extra fields
	// [screens], [screen.<id>], [track.<id>]: the module's loading and menu screens and their music
	// (EditorScreens; game repo docs/dead-air/MODDING.md, "Screens and music")
	enum { scrLoading = 1, scrMenu = 2, scrPause = 4, scrAll = 7 };
	enum { scrHideBar = 1, scrHidePercent = 2, scrHideStage = 4, scrHideTip = 8 };
	enum { scrCover = 0, scrContain = 1, scrStretch = 2 };
	struct SScreenLayer
	{
		xr_string		file;			// project-relative, '\\' separators; empty: nothing put on it yet
		xr_string		texture;		// a shader's own picture
		int				fit = scrCover;
		float			opacity = 1.f;
		float			x = 0.f, y = 0.f, w = 100.f, h = 100.f;	// per cent of the screen
		bool			sound = true;
	};
	struct SScreen
	{
		xr_string		id;
		u32				show = scrAll;
		xr_vector<xr_string> levels, modes;
		float			weight = 1.f;
		xr_string		music = "any";	// any | none | video | tracks
		xr_vector<xr_string> tracks;	// music == "tracks"
		u32				hide = 0;
		xr_vector<SScreenLayer> layers;	// bottom first
	};
	struct STrack
	{
		xr_string		id;
		xr_string		file;
		u32				play = 0;		// the surfaces whose shuffle it joins
		float			weight = 1.f;
		float			volume = 1.f;
	};
	struct SScreens
	{
		// [0] the loading screen, [1] the menu
		bool			only_screens[2] = { false, false };
		bool			only_music[2] = { false, false };
		bool			next_track[2] = { false, false };
		xr_vector<SScreen> screens;
		xr_vector<STrack> tracks;
	};

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
		// The Mod Browser (game repo docs/dead-air/MOD_CATALOG.md 6): the picture next to the
		// name - a 256 x 256 DXT1/DXT5 DDS inside the module, '\' separators - and the tags of the
		// fixed vocabulary (TagVocabulary), in the order the author gave them
		xr_string				thumbnail;
		xr_vector<xr_string>	tags;
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
		// [start_kit]: what a new game of the mode begins with (EditorWorld)
		SStartKit				start_kit;
		// [vars]: the global variables every quest of the module shares, in the author's order
		xr_vector<SGlobalVar>	vars;
		// [pda] layout: the module's own PDA interface, a project-relative .pdaasset
		// (EditorPda); empty = the module leaves the PDA alone
		xr_string				pda_layout;
		xr_vector<xr_string>	pda_extra;		// unknown [pda] lines
		// [commands]: the console commands the module adds, in the author's order
		xr_vector<SCommand>		commands;
		// the module's screens and music (EditorScreens)
		SScreens				screens;
		// [module] disable_game_updates: the game stops updating itself and takes no bug reports from whoever has
		// the module. A last resort: the author has to mean it.
		bool					disable_game_updates = false;
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

	ECORE_API bool	Load				(LPCSTR project_root, SManifest& m);
	ECORE_API bool	Save				(LPCSTR project_root, const SManifest& m);
	// bumped by every Save: a window showing part of the manifest re-reads it when
	// this moves instead of asking the disk every frame
	ECORE_API u32	ManifestGeneration	();
	// the active project's [vars], re-read only when the generation moves - or after
	// InvalidateGlobals, for a mod.ltx that changed on disk behind the editor's back
	ECORE_API const xr_vector<SGlobalVar>& ProjectGlobals();
	ECORE_API void	InvalidateGlobals	();
	// letters, digits and _: what the runtime reads as a name
	ECORE_API bool	ValidGlobalName		(LPCSTR name);

	// ---- console commands ([commands]) ---------------------------------------------
	// a-z, 0-9 and _: the console finds a command by its exact name and ends it at a space
	ECORE_API bool	ValidCommandName	(LPCSTR name);
	// <script>.<function>: two Lua names
	ECORE_API bool	ValidCommandFunction(LPCSTR spec);
	// What a command may call: `function name(` and `name = function(` of the project's
	// gamedata\scripts\*.script as <script>.<function>, sorted. A script the build leaves out
	// has none; `local function` stays inside its script.
	ECORE_API void	ScriptFunctions		(LPCSTR project_root, xr_vector<xr_string>& out);
	// [commands] the modules of the linked game declare, as {command, module id}, the module
	// `self_id` left out
	ECORE_API void	InstalledCommands	(LPCSTR self_id, xr_vector<std::pair<xr_string, xr_string> >& out);
	// The build's check of [commands]: E401 a bad name, E402 a name twice, E403 a function that is
	// not <script>.<function>, E404 one the project's scripts lack, E405 a [command.<name>] with no
	// [commands] line, E406 a name the game has; W406 a name of a family of the game's own,
	// W407 a name another module of the linked game declares, W408 text outside cp1251.
	ECORE_API void	ValidateCommands	(LPCSTR project_root, const SManifest& m, xr_vector<xr_string>& problems);

	// ---- the Mod Browser's picture and tags (game repo MOD_CATALOG 6) ------------
	// `value` names what [module] thumbnail becomes: "" = none (the editor's own
	// thumbnail.dds goes with it), a .dds of the project that already is a thumbnail,
	// or a picture anywhere (PNG, JPG, TGA, BMP) that is turned into <project>\
	// thumbnail.dds. Only `m` is changed; the caller saves it.
	ECORE_API bool	ApplyThumbnail		(LPCSTR project_root, LPCSTR value, SManifest& m, xr_string& err);
	// a comma list of the tag vocabulary; "" = none, an unknown word is an error
	ECORE_API bool	ApplyTags			(LPCSTR value, SManifest& m, xr_string& err);

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
		// the descriptor carries a [card] signed with the author key (Mod Browser, MOD_CATALOG 4);
		// key_id names the key, empty when the release went out unsigned
		bool					signed_;
		xr_string				key_id;
		// The Mod Browser's content rules over the staged files (MOD_CATALOG 8, 10.3): what the
		// catalog refuses and what its review looks at. Reported only - a module outside the
		// catalog ships what it likes, so the release is written either way.
		xr_vector<xr_string>	catalog_block;
		xr_vector<xr_string>	catalog_review;

								SPackageResult	() : files(0), unpacked(0), packed(0), signed_(false) {}
	};

	// A build, an export and a packaging run beside the editor, one at a time: the checks in slices between frames (they
	// read what the open tabs read), the files on a thread of their own. The Task Center shows it, Stop stops it.
	//
	// jkBuildIntoGame / jkBuildToFolder: mod.ltx + module dirs -> <target>\modules\<id>. The build is written beside the
	// last one and takes its place in one step, so a build that fails or is stopped leaves the last one as it was. Files
	// the target holds and the project does not go with it: when those exist and `clean` is false the job ends with the
	// list of them in `stale_names`, so the caller can ask first - the target may be someone else's module that shares
	// the id.
	// jkExportFlat: gamedata content -> <target>\gamedata_<id> + compat report.
	// jkPackage: a release for the game's module updater: <id>.update.ltx, the file index <id>-<version>.files and package
	// zip(s) holding the COMPLETE module under modules/<id>/. The index says where inside a package every file's data
	// sits, so the game downloads only the files that differ. Names, descriptor and index format and package layout are a
	// frozen contract with the game - the normative text is docs/dead-air/MOD_UPDATES.md (section 2) in the game repo. The
	// module is staged by the build above, so everything a module build refuses is refused here too. `out_dir` empty =
	// <project>\releases\<version>; `requires_game` empty = the key is left out. Files an earlier packaging of the same
	// id+version left in `out_dir` are replaced (deleted) without asking - the UI asks first.
	enum EJobKind { jkBuildIntoGame, jkBuildToFolder, jkExportFlat, jkPackage };

	struct SJobResult
	{
		bool					ok			= false;
		bool					cancelled	= false;
		int						files		= 0;
		xr_string				path;				// the folder written, or the one that refused
		xr_string				err;
		xr_string				warnings;			// the problems that did not stop the build
		xr_string				bake_note;			// what the scene's bake did
		xr_vector<xr_string>	left_out;			// what the build passed by: tooling, [exclude], links, project-relative
		xr_vector<xr_string>	stale_names;		// the clean build's refusal: the first ten of the files it would delete
		int						stale_count	= 0;
		SPackageResult			package;			// jkPackage
	};

	struct SJobRequest
	{
		EJobKind				kind		= jkBuildIntoGame;
		xr_string				target;				// the folder a build goes to
		bool					clean		= false;	// the author agreed to the files going
		xr_string				out_dir, requires_game;	// jkPackage
		bool					from_mcp	= false;	// a Task Center row that opens nothing, and nothing asked in the checks
		// the Task Center's row
		xr_string				key, title, subtitle, stop_label, stop_tip;
		xr_vector<xr_string>	steps;				// the first three are Check, Copy and Install or Pack
		// on the main thread, when the job has ended, whichever way
		std::function<void(const SJobResult&)>	done;
	};

	ECORE_API bool	StartJob			(const SJobRequest& request, xr_string& err);
	ECORE_API bool	JobRunning			();
	ECORE_API void	StopJob				();
	// from the editor's loop, drawn or not
	ECORE_API void	JobTick				();
	// the editor closes: stops the job and waits for its thread
	ECORE_API void	JobShutdown			();
	// "Copying files, 38%": what the toolbar's tip says; empty when no job runs
	ECORE_API void	JobStatusText		(char* dst, u32 size);
	// the last job's answer, for an MCP caller held until the job ended
	ECORE_API const SJobResult& LastJobResult();

	// What an earlier packaging of this id + version left in the release folder -
	// the files the next one deletes. A dialog names them first.
	ECORE_API void	ListPackagedFiles	(LPCSTR project_root, LPCSTR out_dir, LPCSTR id, LPCSTR version,
										 xr_vector<xr_string>& names);
	// MAJOR.MINOR.PATCH from the version resource of <linked game>\xrEngine.exe
	ECORE_API bool	LinkedGameVersion	(xr_string& out);

	// the Mod Settings tab and the build dialogs (drawn from CLevelMain::OnDrawUI) + Mod menu triggers;
	// a build started here reports in the shared result dialog, MCP builds answer in JSON
	ECORE_API void	DrawUI				();
	// opens the Mod Settings tab on mod.ltx (made when the project has none)
	ECORE_API void	RequestEditManifest	();
	ECORE_API void	RequestExportModule	();
	ECORE_API void	RequestExportFlat	();
	ECORE_API void	RequestPackageRelease();

	// the everyday build: no target picker, no options - the linked game IS
	// the target, so this is a module build straight to <game>\modules\<id>
	ECORE_API void	RequestBuildIntoGame();
	ECORE_API bool	CanBuildIntoGame	();
	// "<game>\modules\<id>" for a menu tooltip; empty when it can't be built
	ECORE_API void	BuildTargetText		(char* dst, u32 size);

	// Scene work lives a layer above this dll, but a build must ship what the
	// author sees - so the editor registers a bake here and every build path
	// (menu, modal, MCP) runs it first, on the main thread. The note goes into the result.
	typedef void (*TPreBuildBake)(xr_string& note);
	ECORE_API void	SetPreBuildBake		(TPreBuildBake fn);
	ECORE_API void	RunPreBuildBake		(xr_string& note);

	// MCP command handlers: fill the full JSON response; raw = request line
	ECORE_API void	McpManifest			(xr_string& out);
	ECORE_API void	McpSetManifest		(LPCSTR raw, xr_string& out);
	ECORE_API void	McpExport			(LPCSTR raw, xr_string& out);
	// {"path":"docs","excluded":true|false} - the browser's Exclude from Build
	ECORE_API void	McpExclude			(LPCSTR raw, xr_string& out);
	// mod_commands: without "name" the commands, the functions they may call and the problems;
	// {"name","function","help","args","cheat"} adds or changes one, "remove":true takes it out
	ECORE_API void	McpCommands			(LPCSTR raw, xr_string& out);
	ECORE_API void	McpPackage			(LPCSTR raw, xr_string& out);
	// mod_settings_view: {"page":"general|mod_browser|game_mode|relations|build_contents|pda|commands"} opens the
	// Mod Settings tab on that page; without a page it only reports: the page, dirty, each page's state
	// and the problems the form shows
	ECORE_API void	McpSettingsView		(LPCSTR raw, xr_string& out);
}
