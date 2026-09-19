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
	ECORE_API bool	Export				(LPCSTR project_root, LPCSTR target_root, bool flat,
										 int& files, xr_string& out_path, xr_string& err,
										 bool confirmed = false, xr_string* warnings = 0);

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
	// the same id+version left in `out_dir` are replaced (Recycle Bin) without
	// asking - the UI asks first. Callers run RunPreBuildBake, like every build.
	ECORE_API bool	PackageRelease		(LPCSTR project_root, LPCSTR out_dir, LPCSTR requires_game,
										 SPackageResult& result, xr_string& err, xr_string* warnings = 0);
	// What an earlier packaging of this id + version left in the release folder -
	// the files the next one sends to the Recycle Bin. A dialog names them first.
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
	ECORE_API void	McpPackage			(LPCSTR raw, xr_string& out);
}
