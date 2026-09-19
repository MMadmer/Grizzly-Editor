#pragma once
#include <atomic>
#include <functional>
#include <mutex>

// GitHub side of Mod > Publish Release: where the account token comes from, the
// device sign-in, and the publication of a packaged release. Everything here
// blocks on the network and is meant for a worker thread; nothing touches the
// editor state or ImGui.
//
// A release is published the way the game expects to find one (game repo,
// docs/dead-air/MOD_UPDATES.md, section 2): descriptor, file index and packages
// go into a DRAFT, every asset is checked against what was packaged, and only
// then the draft becomes the latest release. The game reads the latest release
// alone, so it never sees one that is half there.

namespace EditorGithub
{
	// Breaks a network call that is blocked on another thread.
	class CCancel
	{
	public:
						CCancel		() : m_Flag(false), m_Request(0) {}
		void			Cancel		();
		void			Reset		()			{ m_Flag = false; }
		bool			Cancelled	() const	{ return m_Flag; }
		// the call in flight; Cancel closes it, which fails the blocked WinHTTP call
		void			Attach		(void* request);
		// false = Cancel already closed it and the caller must not close it again
		bool			Detach		();
	private:
		std::atomic<bool>	m_Flag;
		std::mutex			m_Lock;
		void*				m_Request;
	};

	//--------------------------------------------------------------------------
	// account
	//--------------------------------------------------------------------------
	enum EAccountSource
	{
		asNone,
		asEnvironment,		// XFINED_GITHUB_TOKEN, GH_TOKEN, GITHUB_TOKEN
		asSaved,			// Windows Credential Manager, written by the editor
		asGithubCli,		// gh auth token
	};

	struct SAccount
	{
		EAccountSource	source;
		xr_string		token;
		xr_string		detail;		// the variable that supplied it

						SAccount	() : source(asNone) {}
						~SAccount	();		// wipes the token
	};

	// Environment first (an explicit override), then the token the editor saved,
	// then the GitHub CLI. Starts gh when it has to - a fraction of a second.
	void	ResolveAccount	(SAccount& out);
	LPCSTR	SourceText		(EAccountSource source);
	// what a token looks like; keeps a pasted sentence out of the credential store
	bool	PlausibleToken	(LPCSTR token);
	bool	SaveToken		(LPCSTR token, xr_string& err);
	void	ForgetToken		();
	void	WipeString		(xr_string& s);

	// Device sign-in needs the client id of a registered GitHub OAuth app.
	bool	DeviceSignInAvailable();

	struct SDeviceCode
	{
		xr_string		device_code;
		xr_string		user_code;			// what the author types on GitHub
		xr_string		verification_uri;
		u32				interval;			// seconds between polls
		u32				expires_in;

						SDeviceCode	() : interval(5), expires_in(900) {}
	};

	bool	DeviceBegin		(CCancel& cancel, SDeviceCode& out, xr_string& err);
	// 1 = signed in (token filled), 0 = not yet - poll again after code.interval, -1 = failed
	int		DevicePoll		(CCancel& cancel, SDeviceCode& code, xr_string& token, xr_string& err);

	//--------------------------------------------------------------------------
	// publication
	//--------------------------------------------------------------------------
	struct SAsset
	{
		xr_string		path;		// file on disk
		xr_string		name;		// asset name, exactly as the descriptor spells it
		u64				size;
		xr_string		sha256;		// lowercase hex

						SAsset		() : size(0) {}
	};

	struct SPublishRequest
	{
		xr_string			repo;			// owner/repo
		xr_string			module_id;
		xr_string			version;
		xr_string			tag;
		xr_string			title;
		xr_string			notes;
		bool				prerelease;
		bool				latest;
		xr_string			token;
		xr_vector<SAsset>	assets;			// packages, index, descriptor
		xr_string			descriptor;		// path of <id>.update.ltx, for the final check

							SPublishRequest	() : prerelease(false), latest(true) {}
	};

	struct SPublishResult
	{
		xr_string		url;			// the release page
		xr_string		warnings;
		bool			served;			// the game's own URL already returns the new descriptor

						SPublishResult	() : served(false) {}
	};

	struct IPublishSink
	{
		virtual void	Stage	(LPCSTR text) = 0;
		virtual void	Upload	(LPCSTR asset, u64 sent, u64 total) = 0;
	};

	// Leaves a draft behind when it fails or is cancelled before the last step;
	// the next attempt for the same tag picks that draft up again.
	bool	Publish			(const SPublishRequest& request, CCancel& cancel, IPublishSink& sink,
							 SPublishResult& result, xr_string& err);

	// owner/repo -> https://github.com/owner/repo/releases (QA: the mock's)
	xr_string ReleasesPage	(LPCSTR repo);

	//--------------------------------------------------------------------------
	// what is already out there
	//--------------------------------------------------------------------------
	struct SReleaseInfo
	{
		u64						id;
		xr_string				tag;
		xr_string				title;
		xr_string				notes;			// the release body, Markdown
		xr_string				url;
		xr_string				published;		// "2026-09-19 12:34" UTC; empty for a draft
		bool					draft;
		bool					prerelease;
		bool					latest;			// the one the game reads
		xr_vector<xr_string>	assets;
		u64						bytes;			// all assets together

								SReleaseInfo	() : id(0), draft(false), prerelease(false), latest(false), bytes(0) {}
	};

	// newest first, drafts included - those are what a failed attempt leaves behind
	bool	ListReleases	(const xr_string& repo, const xr_string& token, CCancel& cancel,
							 xr_vector<SReleaseInfo>& out, xr_string& err);

	// Title and notes of a release that is already out; null = keep what it has.
	// Touches nothing else: not the assets, not the tag, and not which release is
	// the latest - GitHub's default for an edit would move that mark onto the
	// edited release, so the request always states where the mark is now.
	bool	EditRelease		(const xr_string& repo, const xr_string& token, u64 id, const xr_string* title,
							 const xr_string* notes, CCancel& cancel, xr_string& tag, xr_string& err);

	struct SDeleteResult
	{
		xr_string		tag;
		bool			draft;			// it was one: no tag to remove
		bool			tag_removed;
		xr_string		tag_note;		// why the tag is still there, when it is

						SDeleteResult	() : draft(false), tag_removed(false) {}
	};

	// The release goes first and its git tag second: a published release whose tag
	// vanishes turns into a draft instead of going away. A draft owns no tag, so
	// none is touched for it. GitHub keeps nothing of either - there is no undo.
	// true = the release is gone; the tag's fate is in `result`.
	bool	DeleteRelease	(const xr_string& repo, const xr_string& token, u64 id, CCancel& cancel,
							 SDeleteResult& result, xr_string& err);
}
