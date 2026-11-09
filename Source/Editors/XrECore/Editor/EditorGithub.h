#pragma once
#include <atomic>
#include <functional>
#include <mutex>

// The network side of the editor's release tools: where the GitHub account token
// comes from, the device sign-in, the publication of a packaged release, the
// Mod Browser submission (an issue of the catalog) and VirusTotal with the
// author's own key. Everything here blocks on the network and is meant
// for a worker thread; nothing touches the editor state or ImGui.
//
// A release is published the way the game expects to find one (game repo,
// docs/dead-air/MOD_UPDATES.md, section 2): descriptor, file index and packages
// go into a DRAFT, every asset is checked against what was packaged, and only
// then the draft becomes the latest release. The game reads the latest release
// alone, so it never sees one that is half there.

namespace EditorGithub
{
	// -qa_github: every address below points at local mocks (GRIZZLY_* variables)
	bool	QaRun			();

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
		asEnvironment,		// GRIZZLY_GITHUB_TOKEN, GH_TOKEN, GITHUB_TOKEN
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
	// starts reading the GitHub CLI's answer, so that the first dialog that asks for the account has it
	void	WarmAccount		();
	// the editor closes
	void	StopAccountRead	();
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

	//--------------------------------------------------------------------------
	// public files: what the game itself reads, fetched the way it fetches them
	//--------------------------------------------------------------------------
	// An anonymous GET that follows redirects. No token travels with it, whatever
	// the address: a release asset, the catalog index, a website.
	bool	FetchPublic		(const xr_string& url, u64 limit, CCancel& cancel, u32& status, xr_string& body,
							 xr_string& err);
	// the address the game downloads `name` of the latest release of `repo` from
	xr_string LatestDownload	(const xr_string& repo, const xr_string& name);
	// https://github.com/<repo> (QA: the mock's)
	xr_string RepoPage		(const xr_string& repo);

	//--------------------------------------------------------------------------
	// the Mod Browser catalog (game repo docs/dead-air/MOD_CATALOG.md 10)
	//--------------------------------------------------------------------------
	// owner/repo the editor submits to (QA: GRIZZLY_CATALOG_REPO)
	xr_string CatalogRepo	();
	// the base URLs the catalog's public files are served from, in the order to try
	void	CatalogMirrors	(xr_vector<xr_string>& out);
	// the catalog's signed index from the first mirror that answers, not verified: it informs the
	// author, and the catalog's own checks read it again
	bool	FetchCatalogIndex	(CCancel& cancel, xr_string& text);

	// the account's login name (GET /user)
	bool	AccountLogin	(const xr_string& token, CCancel& cancel, xr_string& login, xr_string& err);

	// A submission is an issue of the catalog, "Submit <id>", opened by the account that owns the
	// module's repository. The catalog's workflow judges it by itself, comments its verdict and, when
	// it accepts, commits mods/<id>.ltx itself and closes the issue.
	struct SCatalogIssue
	{
		u64			number;			// 0 = none
		xr_string	state;			// open, closed; "none" when there is no submission
		xr_string	url;
		xr_string	login;			// the account that asked
		xr_string	note;			// a submission that sent nothing says why
		// the workflow's newest verdict comment
		xr_string	verdict;		// accepted, waiting, refused; empty = it has not spoken yet
		xr_string	verdict_text;	// the verdict's one plain sentence

					SCatalogIssue	() : number(0), state("none") {}
	};

	struct SCatalogRequest
	{
		xr_string	module_id;
		xr_string	file_text;		// mods/<id>.ltx, byte for byte as the catalog is to commit it
		xr_string	details;		// Markdown after it: the links, the verdicts, the editor's check
		xr_string	token;
	};

	typedef std::function<void(LPCSTR stage)> TStage;

	// The account's open submission takes the new text - an edit makes the catalog judge it again;
	// without one a new issue opens. Nothing is sent when the catalog's default branch already holds
	// this very file. An issue is public from the first second.
	bool	CatalogSubmit	(const SCatalogRequest& rq, CCancel& cancel, const TStage& stage,
							 SCatalogIssue& out, xr_string& err);

	// A withdrawal is an issue too, "Withdraw <id>", and only the account that owns the listed
	// repository may send one: the catalog then marks mods/<id>.ltx withdrawn - the id stays bound to
	// its key - and the Mod Browser stops offering the module. Refused here, with nothing sent, when the
	// catalog's default branch does not hold the module or names another owner; a note and nothing sent
	// when it is withdrawn already. The account's open withdrawal is edited rather than doubled.
	struct SCatalogWithdrawal
	{
		xr_string	module_id;
		xr_string	details;		// Markdown after the ini block, for people
		xr_string	token;
	};
	bool	CatalogWithdraw	(const SCatalogWithdrawal& rq, CCancel& cancel, const TStage& stage,
							 SCatalogIssue& out, xr_string& err);

	enum ECatalogIssue { ciSubmission, ciWithdrawal };
	// the account's newest submission (or withdrawal) of the module, in any state, with the catalog's
	// newest verdict
	bool	CatalogIssue	(const xr_string& module_id, const xr_string& token, CCancel& cancel,
							 SCatalogIssue& out, xr_string& err, ECatalogIssue kind = ciSubmission);

	//--------------------------------------------------------------------------
	// VirusTotal, with the author's own key (MOD_CATALOG 11). The free API takes 4 requests a
	// minute: every request below waits for its turn, whichever job asks. The scan built on
	// them is EditorVirusTotal.
	//--------------------------------------------------------------------------
	// GRIZZLY_VT_KEY or VT_API_KEY first, then the key the editor saved in the Windows
	// Credential Manager. Like the GitHub token it never reaches a log, a file or an answer.
	bool	ReadVtKey		(xr_string& key);
	bool	SaveVtKey		(LPCSTR key, xr_string& err);
	void	ForgetVtKey		();
	bool	PlausibleVtKey	(LPCSTR key);

	struct SVtResult
	{
		xr_string	engine;
		xr_string	category;		// malicious, suspicious, undetected, harmless, ...
	};

	struct SVtReport
	{
		u32						status;		// the HTTP answer; 429 = over the free API's rate
		bool					known;		// VirusTotal has the file
		bool					queued;		// known, analysis not finished: no results yet
		xr_vector<SVtResult>	results;

								SVtReport	() : status(0), known(false), queued(false) {}
	};

	// what VirusTotal says about a file, by its SHA-256
	bool	VtLookup		(const xr_string& sha256, const xr_string& key, CCancel& cancel, SVtReport& out,
							 xr_string& err);
	// Sends a file for analysis (upload_url for one over 32 MB). VirusTotal shares uploaded files
	// with the security industry; results follow within minutes. `status` = 429 when over the rate.
	bool	VtUpload		(const xr_string& path, const xr_string& key, CCancel& cancel, u32& status,
							 xr_string& err);
	// the report page a person opens
	xr_string VtReportPage	(const xr_string& sha256);
	// The largest request VirusTotal takes. The multipart wrapper counts toward it, so a file
	// within a margin of the limit is as good as over it.
	const u64 kVtUploadLimit	= 650ull << 20;
	const u64 kVtWrapping		= 64u << 10;
	inline bool VtTooLarge		(u64 size) { return size + kVtWrapping > kVtUploadLimit; }
}
