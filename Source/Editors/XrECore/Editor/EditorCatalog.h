#pragma once
#include "EditorModManifest.h"

// The Mod Browser's catalog, the author's side (game repo: docs/dead-air/MOD_CATALOG.md): the
// author key file that signs releases (4.1), the [card] Package Release writes into the
// descriptor (4), the content rules a catalog module keeps (8, 10.3), the thumbnail (6.1), the
// tags (6.2) and the VirusTotal verdict (11). The submission - the checks and the issue - is
// EditorCatalogSubmit; the author's VirusTotal scan is EditorVirusTotal.
//
// A key is a FILE the author keeps, never a secret of this computer: Documents\XFined Editor\
// Keys\author.xkey unless the author picks another place, never inside a project. The editor
// remembers where it is and asks for its password once per session. A -qa_github run keeps
// both in XFINED_CATALOG_HOME instead, so a test never meets the author's own key.

namespace EditorCatalog
{
	struct SKeyState
	{
		xr_string	path;			// where the key file is, or where a new one would go
		bool		exists;
		bool		locked;			// protected by a password not given yet this session
		xr_string	public_key;		// p256:...; empty when the file does not read
		xr_string	key_id;			// 16 hex
		xr_string	error;

					SKeyState	() : exists(false), locked(false) {}
	};

	ECORE_API SKeyState	KeyState		();
	// KeyState for a dialog that draws it every frame: the file is read at most once a second,
	// and at once after this unit changed anything about the key. Main thread only.
	ECORE_API SKeyState	KeyStateForUI	();
	ECORE_API xr_string	DefaultKeyPath	();
	// A new key in a new file, and the path remembered. Never overwrites a file.
	ECORE_API bool		CreateKey		(LPCSTR path, LPCSTR password, xr_string& err);
	// Another copy of the key - from another computer, a stick - is the one used from now on.
	ECORE_API bool		UseKeyFile		(LPCSTR path, xr_string& err);
	// The password of a protected key file; the key stays open until the editor closes.
	ECORE_API bool		Unlock			(LPCSTR password, xr_string& err);
	// `body` followed by its [signature] section (MOD_CATALOG 3.1), in the body's own line ends.
	ECORE_API bool		Sign			(const xr_string& body, xr_string& out, xr_string& err);
	// A signed text (3.1) checked against the key it has to be signed with. `signer` names the
	// key id the signature claims, whatever the outcome; empty when the text is not signed.
	ECORE_API bool		Verify			(const xr_string& text, const xr_string& public_key, xr_string& signer,
										 xr_string& err);
	// the editor is closing: the open key is wiped
	ECORE_API void		Shutdown		();
	// One line for a release dialog: what the release will be signed with, a password field when
	// the key is locked, a hint when there is none. `id` keeps the widgets apart.
	ECORE_API void		DrawKeyLine		(LPCSTR id);

	// A file of the release the card names: path as in the file index, size, SHA-256.
	struct SCardFile
	{
		xr_string	path;
		u64			size;
		xr_string	sha256;

					SCardFile	() : size(0) {}
	};
	// The [card] section, starting with its blank line: the manifest with every string table id
	// resolved from the project (name@eng and the like where English text exists), the mode, the
	// tags, the thumbnail and the manifest's own hash.
	ECORE_API xr_string	CardSection		(LPCSTR project_root, const EditorMod::SManifest& m,
										 const SCardFile& manifest, const SCardFile* thumbnail);

	// MOD_CATALOG 6.2, in its order
	ECORE_API const xr_vector<xr_string>& TagVocabulary();
	// the tag's English label for the editor's UI
	ECORE_API LPCSTR	TagLabel		(LPCSTR tag);
	ECORE_API bool		KnownTag		(LPCSTR tag);

	// MOD_CATALOG 7.4: the module's own page on AP-PRO or ModDB, not the site or an index of it
	ECORE_API bool		WebsiteIsModulePage	(LPCSTR url);

	// What a check found: the listing requirements (7), the content (8) and the scripts (10.3).
	// lvRequired is a requirement not met yet that the submission meets by itself - the VirusTotal
	// report its scan waits for - so it does not block the Submit button the way lvBlock does.
	enum ELevel { lvOk, lvNote, lvReview, lvRequired, lvBlock };
	struct SFinding
	{
		ELevel		level;
		xr_string	code;		// what was checked: key, release, content, script, ...
		xr_string	text;

					SFinding	() : level(lvOk) {}
					SFinding	(ELevel l, LPCSTR c, const xr_string& t) : level(l), code(c), text(t) {}
	};
	ECORE_API LPCSTR	LevelName		(ELevel level);

	// MOD_CATALOG 8, the same rules the game keeps when it installs
	ECORE_API bool		PathAllowed		(LPCSTR path, xr_string& why);
	ECORE_API bool		BytesAllowed	(const void* data, size_t size, xr_string& why);
	// One file of a module against 8 and, for a script, 10.3 - what the catalog's check reports.
	// `path` is module-relative with '/'; a non-script needs only its first bytes (ContentHead).
	ECORE_API void		CheckContent	(LPCSTR path, const void* data, size_t size, xr_vector<SFinding>& out);
	// how much of a file CheckContent reads: all of a script, the head of anything else
	ECORE_API size_t	ContentHead		(LPCSTR path, size_t size);
	// the head read so far (`got` of `size` bytes) may point further: an MZ header names where its
	// PE header is. The bytes CheckContent needs, `got` when the head is enough.
	ECORE_API size_t	ContentNeed		(const void* head, size_t got, size_t size);
	// 10.3 over one script: what only a module that runs unprotected would do
	ECORE_API void		ScriptReport	(LPCSTR path, const char* text, size_t size, xr_vector<SFinding>& out);

	// MOD_CATALOG 6.1: 256 x 256, DXT1 or DXT5, one mip level or all nine
	ECORE_API bool		ValidThumbnail	(const void* data, size_t size);
	// Any picture (PNG, JPG, TGA, BMP) to the thumbnail the game takes: the middle square,
	// scaled to 256, DXT1 (DXT5 when the picture has transparency), every mip level.
	ECORE_API bool		MakeThumbnail	(LPCSTR image, LPCSTR dds, xr_string& err);

	// MOD_CATALOG 11: the policy of the catalog index's [virustotal] section and the verdict
	struct SVtPolicy
	{
		xr_vector<xr_string>	trusted;		// lower case
		u32						block_trusted, warn_trusted, block_others, warn_others;

								SVtPolicy	();	// the catalog's own defaults
	};
	struct SVtEngine
	{
		xr_string	engine;
		xr_string	category;		// malicious, suspicious, undetected, harmless, ...
	};
	struct SVtVerdict
	{
		xr_string				verdict;	// clean, warn, block
		u32						engines, trusted_malicious, trusted_suspicious, other_malicious;
		xr_vector<xr_string>	flagged;

								SVtVerdict	() : engines(0), trusted_malicious(0), trusted_suspicious(0), other_malicious(0) {}
	};
	ECORE_API void		ReadVtPolicy	(const xr_string& index_text, SVtPolicy& out);
	ECORE_API SVtVerdict VtVerdict		(const xr_vector<SVtEngine>& results, const SVtPolicy& policy);
	// the index's [mods] line for the module: true when it lists the id, `key` the key text it names
	ECORE_API bool		ListedKey		(const xr_string& index_text, const xr_string& id, xr_string& key);
	// the index's [withdrawn] line: a module its author withdrew, whose id stays bound to `key`
	ECORE_API bool		WithdrawnKey	(const xr_string& index_text, const xr_string& id, xr_string& key);

	// ini text as the catalog files use it: the value of `key` in `section`, trimmed; comments
	// after an unquoted ';' are dropped
	ECORE_API bool		IniValue		(const xr_string& text, LPCSTR section, LPCSTR key, xr_string& out);
	// every key = value of a section, in order
	ECORE_API void		IniSection		(const xr_string& text, LPCSTR section, xr_vector<std::pair<xr_string, xr_string> >& out);
	// a quoted card text, unescaped (\n, \", \\)
	ECORE_API xr_string	Unquote			(const xr_string& value);
	// MOD_UPDATES 1.2 ordering; missing parts count as 0
	ECORE_API int		CompareVersions	(LPCSTR a, LPCSTR b);
}
