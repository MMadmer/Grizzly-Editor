#include "stdafx.h"
#include "EditorModPublish.h"
#include "EditorCatalog.h"
#include "EditorGithub.h"
#include "EditorModManifest.h"
#include "EditorUiContract.h"
#include "EditorProject.h"
#include "EditorVirusTotal.h"
#include "XFinedMCP.h"
#include "ui_main.h"
#include "Nq/NqUtil.h"
#include <shellapi.h>
#include <thread>
#include <ctime>

namespace
{
const u32 kMaxNotes = 120000;	// GitHub stops a release body at 125 000 characters

//------------------------------------------------------------------------------
// json helpers
//------------------------------------------------------------------------------
void JsonAppend(xr_string& out, LPCSTR s)
{
	for (const char* p = s; *p; ++p)
	{
		switch (*p)
		{
		case '"':	out += "\\\""; break;
		case '\\':	out += "\\\\"; break;
		case '\n':	out += "\\n"; break;
		case '\r':	out += "\\r"; break;
		case '\t':	out += "\\t"; break;
		default:
			if ((unsigned char)*p < 0x20)	out += NqUtil::Format("\\u%04x", (unsigned)(unsigned char)*p);
			else							out += *p;
		}
	}
}

void JsonField(xr_string& out, LPCSTR key, LPCSTR value)
{
	out += ",\"";
	out += key;
	out += "\":\"";
	JsonAppend(out, value);
	out += "\"";
}

// bare (unquoted) bool argument: {"prerelease":true}
bool ArgBool(LPCSTR raw, LPCSTR field, bool def)
{
	char pat[64];
	sprintf_s(pat, "\"%s\"", field);
	const char* k = raw ? strstr(raw, pat) : 0;
	if (!k) return def;
	const char* c = strchr(k + xr_strlen(pat), ':');
	if (!c) return def;
	while (*++c == ' ') {}
	return 0 == strncmp(c, "true", 4);
}

//------------------------------------------------------------------------------
// the upload job
//------------------------------------------------------------------------------
enum EJobState { jsIdle, jsScanning, jsUploading, jsDone, jsFailed, jsCancelled };

struct SJobView
{
	EJobState				state;
	xr_string				id, version, repo, tag;
	xr_string				stage;
	u64						sent, total;		// bytes of the whole release
	xr_string				url, error, warnings;
	bool					served;
	EditorVirusTotal::SScan	virustotal;			// none: the Mod Browser does not list the module
	bool					refused;			// the scan kept the release back
	bool					github;				// something was sent to GitHub

	SJobView() : state(jsIdle), sent(0), total(0), served(false), refused(false), github(false) {}
};

// A module the Mod Browser lists has every release scanned first: the catalog only looks packages
// up and holds back a release VirusTotal does not know. What the scan needs, taken with the job.
struct SScanPlan
{
	xr_string								module_id;
	xr_vector<EditorVirusTotal::SPackage>	packages;
	xr_string								vt_key;		// empty = none

	~SScanPlan() { EditorGithub::WipeString(vt_key); }
};

std::mutex				s_JobLock;
SJobView				s_Job;					// guarded by s_JobLock
std::thread				s_JobThread;
EditorGithub::CCancel	s_JobCancel;
bool					s_JobFromUI		= false;
bool					s_JobReported	= true;		// its end reached the log and the result window

class CSink : public EditorGithub::IPublishSink
{
public:
	explicit CSink(const EditorGithub::SPublishRequest& rq)
	{
		u64 before = 0;
		for (u32 i = 0; i < rq.assets.size(); ++i)
		{
			m_Names.push_back(rq.assets[i].name);
			m_Before.push_back(before);
			before += rq.assets[i].size;
		}
	}
	virtual void Stage(LPCSTR text)
	{
		std::lock_guard<std::mutex> guard(s_JobLock);
		s_Job.stage = text;
	}
	virtual void Upload(LPCSTR asset, u64 sent, u64)
	{
		for (u32 i = 0; i < m_Names.size(); ++i)
			if (m_Names[i] == asset)
			{
				std::lock_guard<std::mutex> guard(s_JobLock);
				s_Job.sent = m_Before[i] + sent;
				return;
			}
	}
private:
	xr_vector<xr_string>	m_Names;
	xr_vector<u64>			m_Before;	// bytes of the assets ahead of this one
};

void SetScan(EJobState state, const EditorVirusTotal::SScan& scan)
{
	std::lock_guard<std::mutex> guard(s_JobLock);
	s_Job.state = state;
	s_Job.virustotal = scan;
}

// Before anything is created on GitHub. A module the index does not list is published as ever;
// one it lists - or any, while the index does not answer and there is a key - is scanned, and a
// release of a listed module leaves only with a VirusTotal report of every package, one the catalog
// would not hold back. false = not published, `err` says why.
bool ScanFirst(const SScanPlan& plan, CSink& sink, xr_string& err, xr_string& warnings)
{
	sink.Stage("Reading the Mod Browser catalog");
	xr_string index, key_text;
	const bool reached = EditorGithub::FetchCatalogIndex(s_JobCancel, index);
	if (s_JobCancel.Cancelled()) { err = "cancelled"; return false; }
	const bool listed = reached && EditorCatalog::ListedKey(index, plan.module_id, key_text);
	if (reached && !listed)
	{
		if (EditorCatalog::WithdrawnKey(index, plan.module_id, key_text))
			warnings = plan.module_id + " is withdrawn from the Mod Browser: players who installed it there do not get this "
				"release. Submitting the module again lists it again.";
		return true;
	}

	EditorCatalog::SVtPolicy policy;
	if (reached) EditorCatalog::ReadVtPolicy(index, policy);
	EditorVirusTotal::SScan scan;
	if (plan.vt_key.empty())
	{
		EditorVirusTotal::Scan(plan.packages, plan.vt_key, policy, s_JobCancel, EditorVirusTotal::TStage(), scan);
		SetScan(jsUploading, scan);
		if (!listed)
		{
			warnings = "The Mod Browser catalog did not answer, and without a VirusTotal key nothing was scanned: if this "
				"module is in the Mod Browser, the release is offered there only once VirusTotal knows its packages.";
			return true;
		}
		err = plan.module_id + " is in the Mod Browser, and every release of it is scanned on VirusTotal with your own free "
			"key before it goes out. Add the key in Mod > Submit to Mod Browser (MCP: mod_catalog action=vt_key) - a free "
			"account at virustotal.com has one.";
		return false;
	}

	scan.state = "scanning";
	SetScan(jsScanning, scan);
	EditorVirusTotal::Scan(plan.packages, plan.vt_key, policy, s_JobCancel, [&sink](LPCSTR text) { sink.Stage(text); }, scan);
	SetScan(jsUploading, scan);
	if (s_JobCancel.Cancelled()) { err = "cancelled"; return false; }
	const EditorVirusTotal::SResult* deciding = EditorVirusTotal::Deciding(scan);
	if (!deciding) return true;
	const xr_string what = EditorVirusTotal::Describe(*deciding);
	if (scan.state == "block")
		err = what + ". The Mod Browser would withdraw this release.";
	else if (scan.state == "too_large")
		err = what + ", and the Mod Browser holds back a release VirusTotal cannot know. Package Release keeps every "
			"package under that size unless a single file alone is larger: make that file smaller.";
	else if (EditorVirusTotal::Refuses(scan.state))
		err = what + ". The Mod Browser would hold this release back.";
	// No report yet: a listed module's release reaches players at once through their updates, so it
	// never leaves before VirusTotal has spoken on every package. Without an index nobody can tell
	// whether the catalog lists it, and it goes out with a warning as ever.
	else if (scan.state == "analysing" || scan.state == "unavailable")
	{
		if (listed)
			err = what + ". A release of a module in the Mod Browser goes out only with a VirusTotal report of every package: " +
				(scan.state == "analysing" ? "publish again in a few minutes - VirusTotal has the package already." :
					"publish again when VirusTotal answers.");
		else
			warnings = what + ". The Mod Browser catalog did not answer: if this module is in the Mod Browser, it is offered "
				"there only once VirusTotal has a report of every package.";
	}
	return err.empty();
}

void JobWorker(EditorGithub::SPublishRequest request, SScanPlan plan)
{
	CSink sink(request);
	EditorGithub::SPublishResult result;
	xr_string err, scan_warnings;
	const bool scanned = ScanFirst(plan, sink, err, scan_warnings);
	EditorGithub::WipeString(plan.vt_key);
	bool ok = scanned;
	if (ok)
	{
		{
			std::lock_guard<std::mutex> guard(s_JobLock);
			s_Job.github = true;
		}
		ok = EditorGithub::Publish(request, s_JobCancel, sink, result, err);
	}
	EditorGithub::WipeString(request.token);

	std::lock_guard<std::mutex> guard(s_JobLock);
	s_Job.state		= ok ? jsDone : (err == "cancelled" ? jsCancelled : jsFailed);
	s_Job.url		= result.url;
	s_Job.warnings	= scan_warnings + (scan_warnings.empty() || result.warnings.empty() ? "" : "\n") + result.warnings;
	s_Job.served	= result.served;
	s_Job.error		= ok ? xr_string() : err;
	s_Job.refused	= !scanned && err != "cancelled";
	if (ok) s_Job.sent = s_Job.total;
}

SJobView JobSnapshot()
{
	std::lock_guard<std::mutex> guard(s_JobLock);
	return s_Job;
}

bool Running(EJobState state) { return jsScanning == state || jsUploading == state; }

bool JobRunning() { return Running(JobSnapshot().state); }

LPCSTR StateText(EJobState state)
{
	switch (state)
	{
	case jsScanning:	return "scanning";
	case jsUploading:	return "uploading";
	case jsDone:		return "done";
	case jsFailed:		return "failed";
	case jsCancelled:	return "cancelled";
	}
	return "idle";
}

//------------------------------------------------------------------------------
// UI state
//------------------------------------------------------------------------------
const char* kTitlePublish	= EditorUiContract::kDialogPublishRelease;
const char* kTitleProgress	= EditorUiContract::kDialogPublishProgress;
const char* kTitleEdit		= EditorUiContract::kDialogEditRelease;
const char* kReleasesTab	= "releases";

bool		s_WantDialog	= false;
bool		s_WantProgress	= false;
bool		s_PublishQueued	= false;

xr_string	s_ModuleId;
xr_string	s_ModuleName;
// what the dialog shows of mod.ltx: read-only here, Mod Settings edits them
xr_string	s_Repo;
xr_string	s_Version;
xr_string	s_Requires;					// the linked game's version: what the release requires
u32			s_ManifestGen	= 0;
char		s_Tag[128]		= {};
char		s_Title[256]	= {};
char		s_Notes[16384]	= {};
char		s_TokenInput[300] = {};
bool		s_TagAuto		= true;		// follows the version until the author edits it
xr_string	s_TagFor;					// the version the automatic tag was worked out for
bool		s_TitleAuto		= true;
bool		s_Prerelease	= false;
bool		s_Latest		= true;
xr_vector<xr_string> s_Replaced;		// an earlier packaging of this version, by file
xr_string	s_ReplacedFor;				// the version that list was made for
EditorGithub::SAccount s_Account;
xr_string	s_AccountError;
xr_string	s_ListAskedFor;				// the repository and account the open dialog read the releases of

bool		s_ReleasesOpen	= false;	// the Releases tab is there
bool		s_RefreshWanted	= false;	// a release went out from the window: the tab reads the list again
u64			s_SelectedId	= 0;		// the release the Releases tab's details show

// What a job started from the UI ends with. It is put up from the frame: a job that an MCP status call
// happens to reap still ends in the window its author waits for.
struct SPendingResult
{
	bool										want = false;
	XUI::Tone									tone = XUI::Tone::Info;
	xr_string									headline, url;
	xr_vector<std::pair<xr_string, xr_string>>	rows;
	XUI::Strings								warnings;
};
SPendingResult s_Result;

// the dialog shows where the account comes from; the token itself is read again
// when a job starts and is not kept around in between
void RefreshAccount()
{
	EditorGithub::ResolveAccount(s_Account);
	EditorGithub::WipeString(s_Account.token);
}

void SplitLines(const xr_string& text, XUI::Strings& out)
{
	for (size_t from = 0; from < text.size();)
	{
		size_t to = text.find('\n', from);
		if (to == xr_string::npos) to = text.size();
		if (to > from) out.push_back(text.substr(from, to - from));
		from = to + 1;
	}
}

// The end of a job, seen from the main thread: the log line, and the result
// window when a person started it. An MCP caller reads the status instead.
void Reap()
{
	const SJobView job = JobSnapshot();
	if (Running(job.state) || jsIdle == job.state) return;
	if (s_JobThread.joinable()) s_JobThread.join();
	if (s_JobReported) return;
	s_JobReported = true;

	SPendingResult r;
	r.want = s_JobFromUI;
	if (jsDone == job.state)
	{
		Msg("* [XMS] release %s %s published: %s", job.id.c_str(), job.version.c_str(), job.url.c_str());
		r.tone = XUI::Tone::Success;
		r.headline = NqUtil::Format("Release %s %s is published.", job.id.c_str(), job.version.c_str());
		if (job.served) r.headline += " The game already offers it: players see the update in the Mods menu.";
		r.url = job.url;
		r.rows.push_back(std::make_pair(xr_string("Repository"), job.repo));
		r.rows.push_back(std::make_pair(xr_string("Tag"), job.tag));
		r.rows.push_back(std::make_pair(xr_string("Release page"), job.url));
	}
	else if (jsCancelled == job.state)
	{
		Msg("* [XMS] publishing %s %s cancelled", job.id.c_str(), job.version.c_str());
		r.tone = XUI::Tone::Warning;
		r.headline = job.github ? "Publishing cancelled. Nothing was published; the draft stays on GitHub and the next attempt reuses it." :
			"Publishing cancelled before anything reached GitHub. Nothing was published.";
	}
	else if (job.refused)
	{
		Msg("! [XMS] release %s %s not published: %s", job.id.c_str(), job.version.c_str(), job.error.c_str());
		r.tone = XUI::Tone::Error;
		r.headline = "The release was not published: " + job.error;
	}
	else
	{
		Msg("! [XMS] publishing %s %s failed: %s", job.id.c_str(), job.version.c_str(), job.error.c_str());
		r.tone = XUI::Tone::Error;
		r.headline = "Publishing failed: " + job.error;
	}
	if (!job.warnings.empty())
	{
		Msg("! [XMS] %s", job.warnings.c_str());
		SplitLines(job.warnings, r.warnings);
	}
	s_WantProgress = false;
	if (r.want) s_Result = r;
	// a release that went out from the window shows up in the Releases tab
	if (s_JobFromUI && s_ReleasesOpen) s_RefreshWanted = true;
}

//------------------------------------------------------------------------------
// starting a job - the one path the dialog and MCP share
//------------------------------------------------------------------------------
bool ManageRunning();	// the release list below: publishing and deleting never run side by side

struct SOptions
{
	xr_string	tag, title, notes;
	xr_string	requires_game;
	bool		requires_given;		// false = the linked game's version
	bool		prerelease, latest;

	SOptions() : requires_given(false), prerelease(false), latest(true) {}
};

// Tags this project deleted and when, one per line, in <project>\releases - a folder no build,
// package or content view ever touches. A release deleted and published again under the same
// tag keeps its download address, and GitHub goes on answering that address with the deleted
// files for minutes (seen live: still stale after 40 s, fresh after 8 min). So a tag deleted
// within the last hour is not offered again; under a fresh tag the address is new.
const long long kTagRestSeconds = 3600;

xr_string DeletedTagsFile(LPCSTR project_root)
{
	return xr_string(project_root) + "\\releases\\deleted.tags";
}

bool TagJustDeleted(const xr_string& tag)
{
	FILE* file = 0;
	if (fopen_s(&file, DeletedTagsFile(EditorProject::Root()).c_str(), "rb") || !file) return false;
	const long long now = (long long)time(0);
	char line[300];
	bool recent = false;
	while (!recent && fgets(line, sizeof(line), file))
	{
		char* tab = strchr(line, '\t');
		if (!tab) continue;
		*tab = 0;
		recent = 0 == _stricmp(line, tag.c_str()) && now - _atoi64(tab + 1) < kTagRestSeconds;
	}
	fclose(file);
	return recent;
}

// worker thread: the file is named by the request, the project may have moved on by now
void NoteDeletedTag(const xr_string& tags_file, const xr_string& tag)
{
	if (tags_file.empty() || tag.empty()) return;
	xr_string folder(tags_file);
	const size_t cut = folder.find_last_of('\\');
	if (cut != xr_string::npos) folder.erase(cut);
	::CreateDirectoryA(folder.c_str(), 0);
	FILE* file = 0;
	if (fopen_s(&file, tags_file.c_str(), "ab") || !file) return;
	fprintf(file, "%s\t%lld\n", tag.c_str(), (long long)time(0));
	fclose(file);
}

// v1.0.0 - or v1.0.0-r2, -r3, ... while the plainer ones were deleted a moment ago
xr_string FreshTag(LPCSTR version)
{
	const xr_string base = xr_string("v") + version;
	xr_string tag = base;
	for (u32 n = 2; n < 1000 && TagJustDeleted(tag); ++n)
		tag = base + NqUtil::Format("-r%u", n);
	return tag;
}

// a git tag name, kept to what every tool accepts
bool ValidTag(LPCSTR tag)
{
	const size_t n = xr_strlen(tag);
	if (!n || n > 100 || tag[0] == '-' || tag[0] == '.' || tag[n - 1] == '.' || strstr(tag, "..")) return false;
	for (size_t i = 0; i < n; ++i)
	{
		const char c = tag[i];
		if (!((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '.' || c == '_' || c == '-'))
			return false;
	}
	return true;
}

xr_string DefaultTitle(const xr_string& id, const xr_string& name, LPCSTR version)
{
	return (name.empty() ? id : name) + " " + version;
}

struct SStarted
{
	EditorMod::SPackageResult	package;
	xr_string					bake_note;
	xr_string					warnings;
};

bool Start(const SOptions& options, bool from_ui, SStarted& started, xr_string& err)
{
	err.clear();
	if (!EditorProject::Active())	{ err = "no active project"; return false; }
	Reap();
	if (JobRunning())				{ err = "a release is being published already"; return false; }
	if (ManageRunning())			{ err = "the release list is busy - try again in a moment"; return false; }

	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m)) { err = "mod.ltx not found in the project"; return false; }
	if (m.update_github.empty())
	{
		err = "no GitHub repository in the manifest: set it in Mod Settings (MCP: mod_set_manifest github=owner/repo)";
		return false;
	}
	if (!EditorMod::ValidateGithubRepo(m.update_github.c_str()))
		{ err = "invalid [update] github in mod.ltx: owner/repo expected"; return false; }

	const xr_string tag		= options.tag.empty() ? FreshTag(m.version.c_str()) : options.tag;
	const xr_string title	= options.title.empty() ? DefaultTitle(m.id, m.name, m.version.c_str()) : options.title;
	if (!ValidTag(tag.c_str()))
		{ err = "invalid tag: 1-100 characters of A-Z a-z 0-9 . _ -, not starting with '-' or '.'"; return false; }
	if (title.size() > 200)				{ err = "the title is longer than 200 characters"; return false; }
	if (options.notes.size() > kMaxNotes)	{ err = "the notes are longer than 120 000 characters"; return false; }

	// A release that goes out unsigned while the author has a key would burn its version for
	// the Mod Browser: a published tag is never overwritten. The key is opened first.
	const EditorCatalog::SKeyState key = EditorCatalog::KeyState();
	if (key.exists && key.error.empty() && key.locked)
	{
		err = "the author key is locked: give its password first (Mod > Publish Release, or MCP password=) - "
			  "an unsigned release is one the Mod Browser does not take";
		return false;
	}

	// before the packaging: no point in zipping a module nobody can upload
	EditorGithub::SAccount account;
	EditorGithub::ResolveAccount(account);
	if (EditorGithub::asNone == account.source)
	{
		err = "not signed in to GitHub: sign in from Mod > Publish Release, sign in to the GitHub CLI (gh auth login) "
			  "or set GH_TOKEN";
		return false;
	}

	xr_string game_version = options.requires_game;
	if (!options.requires_given) EditorMod::LinkedGameVersion(game_version);
	{
		SProgressOperation progress(*UI, true);
		EditorMod::RunPreBuildBake(started.bake_note);
		if (!EditorMod::PackageRelease(EditorProject::Root(), "", game_version.c_str(), started.package, err, &started.warnings))
			return false;
	}

	const EditorMod::SPackageResult& res = started.package;
	SScanPlan plan;
	plan.module_id = m.id;
	for (u32 i = 0; i < res.packages.size(); ++i)
	{
		EditorVirusTotal::SPackage package;
		package.name	= res.packages[i].name;
		package.path	= res.out_dir + "\\" + res.packages[i].name;
		package.size	= res.packages[i].size;
		package.sha256	= res.packages[i].sha256;
		plan.packages.push_back(package);
	}
	EditorGithub::ReadVtKey(plan.vt_key);

	EditorGithub::SPublishRequest request;
	request.repo		= m.update_github;
	request.module_id	= m.id;
	request.version		= m.version;
	request.tag			= tag;
	request.title		= title;
	request.notes		= options.notes;
	request.prerelease	= options.prerelease;
	request.latest		= options.latest && !options.prerelease;
	request.token		= account.token;
	request.descriptor	= res.descriptor;

	// the descriptor goes last, as it does on disk: it vouches for the rest
	xr_vector<EditorMod::SPackageFile> files = res.packages;
	files.push_back(res.index);
	files.push_back(res.descriptor_file);
	u64 total = 0;
	for (u32 i = 0; i < files.size(); ++i)
	{
		EditorGithub::SAsset asset;
		asset.name		= files[i].name;
		asset.path		= res.out_dir + "\\" + files[i].name;
		asset.size		= files[i].size;
		asset.sha256	= files[i].sha256;
		request.assets.push_back(asset);
		total += asset.size;
	}

	{
		std::lock_guard<std::mutex> guard(s_JobLock);
		s_Job			= SJobView();
		s_Job.state		= jsUploading;
		s_Job.id		= m.id;
		s_Job.version	= m.version;
		s_Job.repo		= m.update_github;
		s_Job.tag		= tag;
		s_Job.stage		= "Starting";
		s_Job.total		= total;
	}
	s_JobFromUI		= from_ui;
	s_JobReported	= false;
	s_JobCancel.Reset();
	Msg("* [XMS] publishing %s %s to %s as %s (%s)", m.id.c_str(), m.version.c_str(), m.update_github.c_str(),
		tag.c_str(), EditorGithub::SourceText(account.source));
	s_JobThread = std::thread(&JobWorker, std::move(request), plan);
	return true;
}

//------------------------------------------------------------------------------
// device sign-in
//------------------------------------------------------------------------------
enum ESignState { ssIdle, ssStarting, ssWaiting, ssDone, ssFailed };

struct SSignView
{
	ESignState	state;
	xr_string	user_code, uri, error;

	SSignView() : state(ssIdle) {}
};

std::mutex				s_SignLock;
SSignView				s_Sign;				// guarded by s_SignLock
std::thread				s_SignThread;
EditorGithub::CCancel	s_SignCancel;
bool					s_SignShown		= false;	// the code was copied and the browser opened

void SignSet(ESignState state, LPCSTR error = "")
{
	std::lock_guard<std::mutex> guard(s_SignLock);
	s_Sign.state = state;
	s_Sign.error = error;
}

SSignView SignSnapshot()
{
	std::lock_guard<std::mutex> guard(s_SignLock);
	return s_Sign;
}

void SignWorker()
{
	EditorGithub::SDeviceCode code;
	xr_string err, token;
	if (!EditorGithub::DeviceBegin(s_SignCancel, code, err))
	{
		SignSet(s_SignCancel.Cancelled() ? ssIdle : ssFailed, err.c_str());
		return;
	}
	{
		std::lock_guard<std::mutex> guard(s_SignLock);
		s_Sign.state		= ssWaiting;
		s_Sign.user_code	= code.user_code;
		s_Sign.uri			= code.verification_uri;
	}
	const u32 started = ::GetTickCount();
	for (;;)
	{
		// a poll that comes early is answered with slow_down and a longer interval
		for (u32 waited = 0; waited < code.interval * 1000 + 500 && !s_SignCancel.Cancelled(); waited += 100) ::Sleep(100);
		if (s_SignCancel.Cancelled())										{ SignSet(ssIdle); return; }
		if (::GetTickCount() - started > code.expires_in * 1000)				{ SignSet(ssFailed, "The code expired. Start the sign-in again."); return; }
		const int answer = EditorGithub::DevicePoll(s_SignCancel, code, token, err);
		if (!answer) continue;
		if (answer > 0 && EditorGithub::SaveToken(token.c_str(), err))		SignSet(ssDone);
		else																SignSet(s_SignCancel.Cancelled() ? ssIdle : ssFailed, err.c_str());
		EditorGithub::WipeString(token);
		return;
	}
}

void SignStop()
{
	s_SignCancel.Cancel();
	if (s_SignThread.joinable()) s_SignThread.join();
	s_SignCancel.Reset();
	SignSet(ssIdle);
}

void SignStart()
{
	SignStop();
	s_SignShown = false;
	SignSet(ssStarting);
	s_SignThread = std::thread(&SignWorker);
}

//------------------------------------------------------------------------------
// the releases that are already out: the list, editing and deleting one
//------------------------------------------------------------------------------
enum EManageState { msIdle, msRunning, msDone, msFailed };
enum EManageOp { moList, moDelete, moEdit };

// what a job does before it reads the list: nothing, or one release deleted or edited
struct SManageRequest
{
	EManageOp	op;
	u64			id;			// the release, or
	xr_string	tag;		// its tag, when that is what the caller knows
	bool		has_title;	// moEdit: a field that is absent keeps its value
	bool		has_notes;
	xr_string	title;
	xr_string	notes;
	xr_string	tags_file;	// moDelete: where a deleted tag is noted - see FreshTag

	SManageRequest() : op(moList), id(0), has_title(false), has_notes(false) {}
};

struct SManageView
{
	EManageState							state;
	xr_string								repo;
	EManageOp								op;			// what the job in flight does before it lists
	xr_string								error;		// why the last job failed
	xr_string								error_text;	// ...as the tab says it: a sentence naming what failed
	xr_string								note;		// what a deletion did
	bool									tag_kept;	// ...and whether it left the tag behind
	bool									listed;		// `releases` is GitHub's answer, not an empty start
	xr_vector<EditorGithub::SReleaseInfo>	releases;
	u32										generation;	// every change bumps it: the tab copies the view only then

	SManageView() : state(msIdle), op(moList), tag_kept(false), listed(false), generation(0) {}
};

// why the Releases tab has nothing to list
enum EReleasesSetup { rsOk, rsNoProject, rsNoRepo, rsNoAccount };

std::mutex				s_ManageLock;
SManageView				s_Manage;			// guarded by s_ManageLock
std::thread				s_ManageThread;
EditorGithub::CCancel	s_ManageCancel;

SManageView ManageSnapshot()
{
	std::lock_guard<std::mutex> guard(s_ManageLock);
	return s_Manage;
}

bool ManageRunning()
{
	std::lock_guard<std::mutex> guard(s_ManageLock);
	return msRunning == s_Manage.state;
}

void ManageReap()
{
	if (!ManageRunning() && s_ManageThread.joinable()) s_ManageThread.join();
}

// A reason GitHub, the network or a job gave is a clause; these make it the end of a sentence, or one of its own.
xr_string Reason(xr_string text)
{
	while (!text.empty() && (text.back() == '.' || text.back() == ' ')) text.pop_back();
	return text;
}

xr_string Sentence(const xr_string& text)
{
	xr_string out = Reason(text);
	if (!out.empty()) out[0] = (char)toupper((unsigned char)out[0]);
	return out.empty() ? out : out + ".";
}

void SetManageError(const xr_string& err)
{
	std::lock_guard<std::mutex> guard(s_ManageLock);
	s_Manage.error = err;
	s_Manage.error_text = Sentence(err);
	++s_Manage.generation;
}

void ManageWorker(xr_string repo, xr_string token, SManageRequest rq)
{
	xr_string error, note;
	bool tag_kept = false;
	xr_vector<EditorGithub::SReleaseInfo> releases;

	if (moList != rq.op && !rq.id && EditorGithub::ListReleases(repo, token, s_ManageCancel, releases, error))
	{
		// a tag names one published release, but any number of drafts may share it
		u32 matches = 0;
		for (u32 i = 0; i < releases.size(); ++i)
			if (releases[i].tag == rq.tag) { ++matches; rq.id = releases[i].id; }
		if (!matches)
			error = NqUtil::Format("no release carries the tag %s", rq.tag.c_str());
		else if (matches > 1)
		{
			error = NqUtil::Format("%u releases carry the tag %s - name one by its id", matches, rq.tag.c_str());
			rq.id = 0;
		}
	}
	if (error.empty() && rq.id && moDelete == rq.op)
	{
		EditorGithub::SDeleteResult result;
		if (EditorGithub::DeleteRelease(repo, token, rq.id, s_ManageCancel, result, error))
		{
			if (!result.draft) NoteDeletedTag(rq.tags_file, result.tag);
			if (result.draft)
				note = NqUtil::Format("Draft release %s is deleted.", result.tag.c_str());
			else if (result.tag_removed)
				note = NqUtil::Format("Release %s and its tag are deleted.", result.tag.c_str());
			else
			{
				tag_kept = true;
				note = NqUtil::Format("Release %s is deleted, its tag is not: %s", result.tag.c_str(), result.tag_note.c_str());
			}
		}
	}
	if (error.empty() && rq.id && moEdit == rq.op)
	{
		xr_string tag;
		if (EditorGithub::EditRelease(repo, token, rq.id, rq.has_title ? &rq.title : 0, rq.has_notes ? &rq.notes : 0,
				s_ManageCancel, tag, error))
			note = NqUtil::Format("Release %s is updated.", tag.c_str());
	}

	xr_string error_text;
	if (!error.empty())
		error_text = (moDelete == rq.op ? "The release could not be deleted: " : "The release could not be saved: ") + Reason(error) + ".";

	// whatever happened above, the tab shows what GitHub has now
	xr_string list_error;
	const bool listed = EditorGithub::ListReleases(repo, token, s_ManageCancel, releases, list_error);
	if (error.empty() && !list_error.empty())
	{
		error = list_error;
		error_text = "The releases could not be read: " + Reason(list_error) + ".";
	}
	EditorGithub::WipeString(token);

	std::lock_guard<std::mutex> guard(s_ManageLock);
	s_Manage.state		= error.empty() ? msDone : msFailed;
	s_Manage.error		= error;
	s_Manage.error_text	= error_text;
	s_Manage.note		= note;
	s_Manage.tag_kept	= tag_kept;
	if (listed)
	{
		s_Manage.listed		= true;
		s_Manage.releases	= releases;
	}
	++s_Manage.generation;
}

// `setup` says why nothing can be listed at all: no project, no repository, no account
bool ManageStart(const SManageRequest& rq, xr_string& err, EReleasesSetup* setup = 0)
{
	err.clear();
	if (setup) *setup = rsOk;
	if (!EditorProject::Active())
	{
		err = "no active project";
		if (setup) *setup = rsNoProject;
		return false;
	}
	Reap();
	ManageReap();
	if (JobRunning())				{ err = "a release is being published - wait for it to finish"; return false; }
	if (ManageRunning())			{ err = "the release list is busy - try again in a moment"; return false; }

	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m))
	{
		err = "mod.ltx not found in the project";
		if (setup) *setup = rsNoRepo;
		return false;
	}
	if (m.update_github.empty() || !EditorMod::ValidateGithubRepo(m.update_github.c_str()))
	{
		err = "no GitHub repository in the manifest: set it in Mod Settings (MCP: mod_set_manifest github=owner/repo)";
		if (setup) *setup = rsNoRepo;
		return false;
	}
	EditorGithub::SAccount account;
	EditorGithub::ResolveAccount(account);
	if (EditorGithub::asNone == account.source)
	{
		err = "not signed in to GitHub: sign in from Mod > Publish Release, sign in to the GitHub CLI (gh auth login) "
			  "or set GH_TOKEN";
		if (setup) *setup = rsNoAccount;
		return false;
	}

	{
		std::lock_guard<std::mutex> guard(s_ManageLock);
		// another repository's releases are not this one's
		if (s_Manage.repo != m.update_github)
		{
			s_Manage.releases.clear();
			s_Manage.listed = false;
		}
		s_Manage.state		= msRunning;
		s_Manage.repo		= m.update_github;
		s_Manage.op			= rq.op;
		s_Manage.error.clear();
		s_Manage.error_text.clear();
		s_Manage.note.clear();
		s_Manage.tag_kept	= false;
		++s_Manage.generation;
	}
	if (moList != rq.op)
		Msg("* [XMS] %s release %s in %s", moDelete == rq.op ? "deleting" : "editing",
			rq.id ? NqUtil::Format("#%llu", rq.id).c_str() : rq.tag.c_str(), m.update_github.c_str());
	s_ManageCancel.Reset();
	SManageRequest job(rq);
	job.tags_file = DeletedTagsFile(EditorProject::Root());
	s_ManageThread = std::thread(&ManageWorker, m.update_github, account.token, job);
	return true;
}

SManageView		s_Shown;					// the view the windows draw, copied when the worker changed it

const SManageView& ShownView()
{
	std::lock_guard<std::mutex> guard(s_ManageLock);
	if (s_Shown.generation != s_Manage.generation) s_Shown = s_Manage;
	return s_Shown;
}

//------------------------------------------------------------------------------
// shared by the windows
//------------------------------------------------------------------------------
ImU32 Token(XFinedTheme::ColorToken token, float alpha = 1.f)
{
	return XFinedTheme::U32(token, alpha);
}

// GitHub's own pages only: the address of a release comes over the network
bool IsGithubPage(const xr_string& url)
{
	return 0 == strncmp(url.c_str(), "https://github.com/", 19);
}

void OpenGithubPage(const xr_string& url)
{
	if (IsGithubPage(url))
		::ShellExecuteA(0, "open", url.c_str(), 0, 0, SW_SHOWNORMAL);
}

// a label over the field it names, in a dialog without a property grid
void FieldLabel(LPCSTR text)
{
	ImGui::PushStyleColor(ImGuiCol_Text, Token(XFinedTheme::ColorToken::TextSecondary));
	ImGui::TextUnformatted(text);
	ImGui::PopStyleColor();
}

//------------------------------------------------------------------------------
// the Publish Release dialog
//------------------------------------------------------------------------------
// what the dialog shows of mod.ltx, read again whenever the manifest changes under it
void ReadManifestFields()
{
	s_ManifestGen = EditorMod::ManifestGeneration();
	EditorMod::SManifest m;
	EditorMod::Load(EditorProject::Root(), m);
	s_ModuleId		= m.id;
	s_ModuleName	= m.name;
	s_Repo			= m.update_github;
	s_Version		= m.version;
}

void RefreshReplaced()
{
	if (s_ReplacedFor == s_Version) return;
	s_ReplacedFor = s_Version;
	EditorMod::ListPackagedFiles(EditorProject::Root(), "", s_ModuleId.c_str(), s_Version.c_str(), s_Replaced);
}

void RunDeferredPublish()
{
	s_PublishQueued = false;
	SOptions options;
	options.tag				= s_Tag;
	options.title			= s_Title;
	options.notes			= s_Notes;
	options.requires_game	= s_Requires;
	options.requires_given	= true;
	options.prerelease		= s_Prerelease;
	options.latest			= s_Latest;

	SStarted started;
	xr_string err;
	if (Start(options, true, started, err))
	{
		s_WantProgress = true;
		return;
	}
	if (err == "cancelled")
		XUI::Notice(XUI::Tone::Warning, "Publish Release", "Packaging cancelled. Nothing was published.");
	else
		XUI::Notice(XUI::Tone::Error, "Publish Release", ("Cannot publish the release: " + err + ".").c_str());
}

// a sign-in nobody is watching and a token nobody is going to save
void DialogCleanup()
{
	SignStop();
	SecureZeroMemory(s_TokenInput, sizeof(s_TokenInput));
}

// the author key is there and wants its password before anything can be signed
bool KeyLocked()
{
	return EditorCatalog::KeyWantsPassword();
}

// MAJOR.MINOR.PATCH, what PackageRelease accepts for requires_game
bool GameVersionOk(LPCSTR v)
{
	int dots = 0;
	for (const char* p = v; *p; ++p) dots += ('.' == *p);
	return 2 == dots && EditorMod::ValidateVersion(v);
}

// v1.2.0 and the fresh tags of the same version (v1.2.0-r2, ...): EditorGithub::Publish refuses a version that is
// out under any of them
bool IsVersionTag(const xr_string& tag, const xr_string& version_tag)
{
	if (0 != tag.compare(0, version_tag.size(), version_tag)) return false;
	LPCSTR rest = tag.c_str() + version_tag.size();
	if (!rest[0]) return true;
	if ('-' != rest[0] || 'r' != rest[1] || !rest[2]) return false;
	for (LPCSTR c = rest + 2; *c; ++c)
		if (*c < '0' || *c > '9') return false;
	return true;
}

// What GitHub would refuse after the packaging, known before the click: a published release of this version, or one
// that carries the tag. Nothing while the dialog has no list of the repository's releases.
struct SPublishedClash
{
	bool		found = false;
	bool		version = false;	// the version is out; otherwise only the tag is taken
	u64			id = 0;
	xr_string	tag;
};

SPublishedClash PublishedClash(const SManageView& view)
{
	SPublishedClash clash;
	if (!view.listed || view.repo != s_Repo) return clash;
	const bool version_ok = EditorMod::ValidateVersion(s_Version.c_str());
	const xr_string version_tag = "v" + s_Version;
	for (const EditorGithub::SReleaseInfo& r : view.releases)
	{
		if (r.draft) continue;
		const bool version = version_ok && IsVersionTag(r.tag, version_tag);
		if (!version && (clash.found || r.tag != s_Tag)) continue;
		clash.found		= true;
		clash.version	= version;
		clash.id		= r.id;
		clash.tag		= r.tag;
		if (version) break;
	}
	return clash;
}

// The dialog reads the releases once it is up, and again for another repository or account: a version that is out
// is listed as a problem before the click instead of refused after the packaging.
void EnsureReleaseList()
{
	if (EditorGithub::asNone == s_Account.source || !EditorMod::ValidateGithubRepo(s_Repo.c_str())) return;
	const xr_string key = s_Repo + "|" + EditorGithub::SourceText(s_Account.source);
	if (key == s_ListAskedFor || JobRunning()) return;
	if (ManageRunning())
	{
		// a list of this repository is on its way already
		const SManageView& view = ShownView();
		if (moList == view.op && view.repo == s_Repo) s_ListAskedFor = key;
		return;
	}
	s_ListAskedFor = key;
	xr_string err;
	if (!ManageStart(SManageRequest(), err)) SetManageError(err);
}

// what a problem sends the author to
enum EPublishFix { fxSettings = 1, fxReleases = 2 };

// every reason the Publish button is off, in the order the dialog shows the fields
void PublishProblems(const SPublishedClash& clash, XUI::Strings& out, u32& fixes)
{
	fixes = 0;
	if (s_Repo.empty())
		out.push_back("No GitHub repository: set one in Mod Settings ([update] github).");
	else if (!EditorMod::ValidateGithubRepo(s_Repo.c_str()))
		out.push_back("The GitHub repository in Mod Settings is not owner/repo: " + s_Repo + ".");
	if (!EditorMod::ValidateVersion(s_Version.c_str()))
		out.push_back("The version in Mod Settings is not valid: 1-4 numbers separated by dots, e.g. 1.2.0.");
	if (!out.empty()) fixes |= fxSettings;
	if (clash.version)
	{
		out.push_back(NqUtil::Format("Version %s is already published as %s, and a published release is never overwritten: raise the "
			"version in Mod Settings, or delete that release in Releases first.", s_Version.c_str(), clash.tag.c_str()));
		fixes |= fxSettings | fxReleases;
	}
	if (!s_Requires.empty() && !GameVersionOk(s_Requires.c_str()))
		out.push_back("The linked game's version " + s_Requires + " is not MAJOR.MINOR.PATCH.");
	if (KeyLocked())
		out.push_back("The author key is locked: give its password below - the release is signed with it.");
	if (!ValidTag(s_Tag))
		out.push_back("The tag takes 1-100 letters, digits, '.', '_' and '-', and starts with neither '-' nor '.'.");
	else if (clash.found && !clash.version)
		out.push_back(NqUtil::Format("Tag %s is already published, and a published release is never overwritten: pick another tag.", s_Tag));
	if (xr_strlen(s_Title) > 200)
		out.push_back("The title is longer than 200 characters.");
	if (EditorGithub::asNone == s_Account.source)
		out.push_back("Not signed in to GitHub: sign in or save a token below.");
	// reading the list is not a problem, a deletion or an edit in flight is
	const SManageView& view = ShownView();
	if (msRunning == view.state && moList != view.op)
		out.push_back("A release is being deleted or edited: wait for it to finish.");
}

// a sign-in that went through while the dialog is up: the account it shows is read again
void ReapSignIn()
{
	if (ssDone != SignSnapshot().state) return;
	if (s_SignThread.joinable()) s_SignThread.join();
	SignSet(ssIdle);
	RefreshAccount();
}

// The account rows of the dialog's grid: where the token comes from, the device sign-in, a pasted token.
void DrawAccountRows()
{
	const SSignView sign = SignSnapshot();
	const bool signed_in = EditorGithub::asNone != s_Account.source;
	const bool signing = ssStarting == sign.state || ssWaiting == sign.state;

	if (XUI::PropertyRow("Account", "Where the GitHub token comes from: an environment variable, the token the editor saved in the "
			"Windows Credential Manager, or the GitHub CLI (gh auth login) - in that order.", signed_in ? XUI::PropReadOnly : XUI::PropRequired))
	{
		xr_string text = signed_in ? xr_string(EditorGithub::SourceText(s_Account.source)) : xr_string("Not signed in");
		if (signed_in && !s_Account.detail.empty()) text += " (" + s_Account.detail + ")";
		XUI::PropertyText(text.c_str());
		if (EditorGithub::DeviceSignInAvailable() && !signing &&
			XUI::PropertyTrailing("##sign_in", XUI::Icon::Link, "Sign in with GitHub: opens GitHub in the browser and asks for a one-time code"))
			SignStart();
		if (EditorGithub::asSaved == s_Account.source && XUI::PropertyTrailing("##forget_token", XUI::Icon::Unlink, "Forget the token the editor saved"))
		{
			EditorGithub::ForgetToken();
			RefreshAccount();
		}
		if (ssFailed == sign.state && !sign.error.empty())
			XUI::PropertyMessage(XUI::Tone::Error, sign.error.c_str());
	}

	if (ssStarting == sign.state && XUI::PropertyRow("Sign-in", "GitHub hands out a one-time code that is entered on its own page."))
	{
		XUI::BusyLine("Contacting GitHub...");
		if (XUI::PropertyTrailing("##cancel_sign_in", XUI::Icon::Close, "Cancel the sign-in")) SignStop();
	}
	if (ssWaiting == sign.state)
	{
		// GitHub's own page, and only that: the address came over the network
		const bool trusted = IsGithubPage(sign.uri);
		if (!s_SignShown)
		{
			s_SignShown = true;
			ImGui::SetClipboardText(sign.user_code.c_str());
			OpenGithubPage(sign.uri);
		}
		if (XUI::PropertyRow("Code", "Enter this code on GitHub. The token GitHub then hands out goes into the Windows Credential Manager.",
				XUI::PropReadOnly))
		{
			ImGui::PushFont(XUI::GetFont(XUI::Font::Mono), XUI::FontSize(XUI::Font::Title));
			ImGui::TextUnformatted(sign.user_code.c_str());
			ImGui::PopFont();
			if (XUI::PropertyTrailing("##cancel_sign_in", XUI::Icon::Close, "Cancel the sign-in")) SignStop();
			if (trusted && XUI::PropertyTrailing("##open_github", XUI::Icon::External, "Open GitHub to enter the code")) OpenGithubPage(sign.uri);
			if (XUI::PropertyTrailing("##copy_code", XUI::Icon::Copy, "Copy the code")) ImGui::SetClipboardText(sign.user_code.c_str());
			XUI::PropertyMessage(XUI::Tone::Muted, "The code is in the clipboard. Waiting for the confirmation on GitHub...");
		}
	}

	if (XUI::PropertyRow("Token", "A personal access token: GitHub > Settings > Developer settings > Fine-grained tokens, this repository, "
			"Contents = Read and write. Kept in the Windows Credential Manager - never in the project."))
	{
		const float button = ImGui::CalcTextSize("Save").x + XUI::Pt(20.f);
		ImGui::SetNextItemWidth(_max(XUI::Em(6.f), ImGui::GetContentRegionAvail().x - button - ImGui::GetStyle().ItemSpacing.x));
		ImGui::InputTextWithHint("##token", signed_in ? "Replace with a personal access token" : "Personal access token", s_TokenInput,
			sizeof(s_TokenInput), ImGuiInputTextFlags_Password);
		ImGui::SameLine();
		if (XUI::Button("Save##token", XUI::Kind::Secondary, s_TokenInput[0] != 0, "Paste a token first."))
		{
			if (EditorGithub::SaveToken(s_TokenInput, s_AccountError))
			{
				s_AccountError.clear();
				RefreshAccount();
			}
			SecureZeroMemory(s_TokenInput, sizeof(s_TokenInput));
		}
		if (!s_AccountError.empty())
			XUI::PropertyMessage(XUI::Tone::Error, s_AccountError.c_str());
	}
}

void DrawDialog()
{
	// the cross, Esc and ui_dialog close clear the flag without passing through here
	static bool was_open = false;
	if (was_open && !s_WantDialog) DialogCleanup();
	was_open = s_WantDialog;
	if (!XUI::BeginDialog(kTitlePublish, &s_WantDialog, XUI::DialogSize::L))
		return;

	if (s_ManifestGen != EditorMod::ManifestGeneration()) ReadManifestFields();
	// the used-tag list is a file: read it when the version changes, not every frame
	if (s_TagAuto && s_TagFor != s_Version)
	{
		s_TagFor = s_Version;
		strncpy_s(s_Tag, sizeof(s_Tag), FreshTag(s_Version.c_str()).c_str(), _TRUNCATE);
	}
	if (s_TitleAuto) strncpy_s(s_Title, sizeof(s_Title), DefaultTitle(s_ModuleId, s_ModuleName, s_Version.c_str()).c_str(), _TRUNCATE);
	RefreshReplaced();
	ReapSignIn();
	EnsureReleaseList();
	const SManageView& view = ShownView();
	const bool reading = msRunning == view.state && moList == view.op;
	const bool list_failed = msFailed == view.state && moList == view.op && view.repo == s_Repo && !view.error.empty();
	const SPublishedClash clash = PublishedClash(view);
	XUI::Strings problems;
	u32 fixes = 0;
	PublishProblems(clash, problems, fixes);
	bool settings = false, releases = false;

	XUI::BeginDialogBody();
	for (const xr_string& problem : problems)
		XUI::StatusText(XUI::Tone::Warning, problem.c_str());
	// where each problem is fixed, by name: Repository and Version are read-only here
	if (fixes & fxSettings)
	{
		settings = XUI::Button("Open Mod Settings", XUI::Kind::Secondary);
		XUI::Tip("Closes this dialog and opens Mod Settings, where the repository and the version are set.");
		if (fixes & fxReleases) ImGui::SameLine();
	}
	if (fixes & fxReleases)
	{
		releases = XUI::Button("Open Releases", XUI::Kind::Secondary);
		XUI::Tip("Closes this dialog and opens the Releases tab on the release of this version.");
	}
	if (list_failed)
		XUI::StatusText(XUI::Tone::Muted, ("The releases of " + s_Repo + " could not be read: " + Reason(view.error) + ". Publishing "
			"checks the version on GitHub again.").c_str());
	if (!problems.empty() || list_failed)
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));

	if (XUI::BeginPropertyGrid("publish"))
	{
		const bool repo_ok = !s_Repo.empty() && EditorMod::ValidateGithubRepo(s_Repo.c_str());
		const bool version_ok = EditorMod::ValidateVersion(s_Version.c_str()) && !clash.version;
		// what goes out, as Mod Settings and the linked game have it: read-only here
		if (XUI::PropertySection("Module"))
		{
			if (XUI::PropertyRow("Name", "The module this release carries: its name and id in Mod Settings.", XUI::PropReadOnly))
			{
				const xr_string name = s_ModuleName.empty() || s_ModuleName == s_ModuleId ? s_ModuleId : s_ModuleName + " (" + s_ModuleId + ")";
				XUI::PropertyText(name.c_str());
			}
			if (XUI::PropertyRow("Repository", "The GitHub repository whose latest release updates this module ([update] github). Mod "
					"Settings sets it.", XUI::PropReadOnly | (repo_ok ? 0 : XUI::PropInvalid)))
			{
				XUI::PropertyText(s_Repo.empty() ? "Not set" : s_Repo.c_str());
				if (repo_ok && XUI::PropertyTrailing("##repo_page", XUI::Icon::External, "Open the repository on GitHub"))
					OpenGithubPage(EditorGithub::RepoPage(s_Repo));
			}
			if (XUI::PropertyRow("Version", "The version of this release ([module] version). Mod Settings sets it. A player is offered the "
					"update when it is higher than the installed one; a version that is published already is never published again.",
					XUI::PropReadOnly | (version_ok ? 0 : XUI::PropInvalid)))
				XUI::PropertyText(s_Version.empty() ? "Not set" : s_Version.c_str());
			if (XUI::PropertyRow("Requires Game", "The lowest Dead Air: Refined version this release runs on: the version of the linked "
					"game. Over MCP, mod_publish requires_game= names another one.", XUI::PropReadOnly))
				XUI::PropertyText(s_Requires.empty() ? "Any version" : s_Requires.c_str());
			// a status line like the rows above, until a locked key asks for its password here
			if (XUI::PropertyRow("Author Key", "The key file that signs the release: the Mod Browser lists signed releases only. Submit to "
					"Mod Browser creates one.", KeyLocked() ? XUI::PropRequired : XUI::PropReadOnly))
				EditorCatalog::DrawKeyLine("publish");
			XUI::EndPropertySection();
		}

		if (XUI::PropertySection("Release"))
		{
			if (XUI::PropertyRow("Tag", "The git tag GitHub creates for the release; the game does not read it. A tag deleted within the "
					"last hour is not offered again: GitHub keeps serving a deleted release's files under a reused tag for minutes.",
					ValidTag(s_Tag) && !(clash.found && !clash.version) ? 0 : XUI::PropInvalid))
				if (ImGui::InputText("##tag", s_Tag, sizeof(s_Tag))) s_TagAuto = false;
			if (XUI::PropertyRow("Title", "The release's title on its GitHub page.", xr_strlen(s_Title) > 200 ? XUI::PropInvalid : 0))
				if (ImGui::InputText("##title", s_Title, sizeof(s_Title))) s_TitleAuto = false;
			if (XUI::PropertyRow("Notes", "What changed, shown on the release page. Markdown."))
				ImGui::InputTextMultiline("##notes", s_Notes, sizeof(s_Notes), ImVec2(0.f, XUI::Em(5.f)));
			if (XUI::PropertyRow("Options", "Players receive only the latest release: the game reads that one alone."))
			{
				ImGui::Checkbox("Pre-release", &s_Prerelease);
				XUI::Tip("Players do not receive pre-releases: the game reads the latest release alone.");
				ImGui::SameLine(0.f, XUI::Pt(16.f));
				ImGui::BeginDisabled(s_Prerelease);
				bool latest = s_Latest && !s_Prerelease;
				if (ImGui::Checkbox("Set as latest", &latest)) s_Latest = latest;
				ImGui::EndDisabled();
				XUI::Tip("The latest release is the one players update to.");
			}
			XUI::EndPropertySection();
		}
		if (XUI::PropertySection("GitHub Account"))
		{
			DrawAccountRows();
			XUI::EndPropertySection();
		}
		XUI::EndPropertyGrid();
	}

	// what goes: an earlier packaging of this version is replaced, file by file
	if (!s_Replaced.empty())
	{
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
		XUI::StatusText(XUI::Tone::Warning, NqUtil::Format("Publishing replaces %u %s of an earlier packaging in releases\\%s:",
			(u32)s_Replaced.size(), s_Replaced.size() == 1 ? "file" : "files", s_Version.c_str()).c_str());
		ImGui::Indent(XUI::Pt(20.f));
		ImGui::PushStyleColor(ImGuiCol_Text, Token(XFinedTheme::ColorToken::TextSecondary));
		const u32 shown = _min((u32)s_Replaced.size(), 10u);
		for (u32 i = 0; i < shown; ++i)
			XUI::TextEllipsis(s_Replaced[i].c_str(), -1.f, XUI::Clip::Middle);
		if (s_Replaced.size() > shown)
			ImGui::TextUnformatted(NqUtil::Format("and %u more", u32(s_Replaced.size() - shown)).c_str());
		ImGui::PopStyleColor();
		ImGui::Unindent(XUI::Pt(20.f));
	}

	const xr_string status = reading ? xr_string() : problems.empty() ? xr_string("Packages, uploads a draft, checks every file, then publishes.") :
		NqUtil::Format("%u %s to fix", (u32)problems.size(), problems.size() == 1 ? "problem" : "problems");
	if (XUI::BeginDialogFooter(status.c_str(), problems.empty() ? XUI::Tone::Muted : XUI::Tone::Warning))
	{
		// the releases that are out decide whether this version may go: nothing is offered before they are known
		if (reading) XUI::FooterBusy("Reading the releases on GitHub...");
		const bool can = problems.empty() && !reading && !s_PublishQueued;
		LPCSTR why = !problems.empty() ? problems.front().c_str() : reading ? "Reading the releases on GitHub first." : "Publishing is starting.";
		if (XUI::DialogButton("Publish", XUI::Kind::Primary, false, can, why))
		{
			s_PublishQueued = UI->DeferUIWork(&RunDeferredPublish);
			s_WantDialog = false;
		}
		if (can)
			XUI::Tip("Packages the module, uploads it to a draft release, checks every file and only then publishes it. A module the "
				"Mod Browser lists is scanned on VirusTotal with your own key first.");
		if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
			s_WantDialog = false;
	}
	XUI::EndDialog();
	if (settings)
	{
		s_WantDialog = false;
		EditorMod::RequestEditManifest();
	}
	else if (releases)
	{
		s_WantDialog = false;
		if (clash.found) s_SelectedId = clash.id;
		EditorPublish::RequestReleases();
	}
}

// the upload goes on in the background; this window follows it until it is hidden
void DrawProgress()
{
	const SJobView job = JobSnapshot();
	if (s_WantProgress && !Running(job.state)) s_WantProgress = false;
	if (!XUI::BeginDialog(kTitleProgress, &s_WantProgress, XUI::DialogSize::M))
		return;
	XUI::BeginDialogBody();
	{
		XUI::FontScope strong(XUI::Font::Strong);
		XUI::TextEllipsis(NqUtil::Format("%s %s to %s", job.id.c_str(), job.version.c_str(), job.repo.c_str()).c_str());
	}
	XUI::BusyLine(job.stage.empty() ? "Starting..." : job.stage.c_str());
	const bool scanning = jsScanning == job.state;
	const float fraction = job.total && !scanning ? float(double(job.sent) / double(job.total)) : 0.f;
	ImGui::PushStyleColor(ImGuiCol_PlotHistogram, Token(XFinedTheme::ColorToken::Accent));
	ImGui::ProgressBar(fraction, ImVec2(ImGui::GetContentRegionAvail().x, XUI::Pt(6.f)), "");
	ImGui::PopStyleColor();
	{
		XUI::FontScope small_font(XUI::Font::Small);
		ImGui::PushStyleColor(ImGuiCol_Text, Token(XFinedTheme::ColorToken::TextMuted));
		const xr_string line = scanning ? xr_string("The Mod Browser lists this module: VirusTotal scans the packages first.") :
			NqUtil::Format("%.1f of %.1f MB \xC2\xB7 %u%%", double(job.sent) / 1048576.0, double(job.total) / 1048576.0,
				job.total ? (u32)(job.sent * 100 / job.total) : 0u);
		ImGui::TextUnformatted(line.c_str());
		ImGui::PopStyleColor();
	}
	if (XUI::BeginDialogFooter())
	{
		const bool stopping = s_JobCancel.Cancelled();
		if (XUI::DialogButton("Stop Publishing", XUI::Kind::Secondary, false, !stopping, "Stopping at the next step."))
			s_JobCancel.Cancel();
		if (!stopping)
			XUI::Tip(scanning ? "Stops the VirusTotal scan. Nothing is published and nothing reaches GitHub." :
				"Stops the upload. Nothing is published; the draft stays on GitHub for the next attempt.");
		if (XUI::DialogButton("Hide", XUI::Kind::Secondary, true))
			s_WantProgress = false;
		XUI::Tip("The upload goes on: the toolbar's Publish button shows its progress and brings this window back.");
	}
	XUI::EndDialog();
}

//------------------------------------------------------------------------------
// the Releases tab: a sortable table of what the repository holds, and the details of one
//------------------------------------------------------------------------------
const u32		kRelistMs		= 60000;	// a tab that comes back after this long reads the list again
EReleasesSetup	s_ReleasesSetup	= rsOk;
u32				s_SetupGen		= 0;		// the manifest the setup was read from
u32				s_ListedAt		= 0;		// when the tab last asked for the list (GetTickCount)
bool			s_WantEdit		= false;
EditorGithub::SReleaseInfo s_EditTarget;	// the release the edit dialog is about
char			s_EditTitle[256] = {};
xr_string		s_EditNotes;				// grows with the text: a release body runs to 125 000 characters

bool ReleasesBusy(const SManageView& view)
{
	return msRunning == view.state || JobRunning();
}

const EditorGithub::SReleaseInfo* Selected(const SManageView& view)
{
	for (const EditorGithub::SReleaseInfo& r : view.releases)
		if (r.id == s_SelectedId) return &r;
	return nullptr;
}

xr_string SizeText(u64 bytes)
{
	if (bytes < 1024) return NqUtil::Format("%llu B", bytes);
	return bytes >= (1u << 20)	? NqUtil::Format("%.1f MB", double(bytes) / 1048576.0)
								: NqUtil::Format("%.0f KB", double(bytes) / 1024.0);
}

// "3 files \xC2\xB7 15 KB"
xr_string FilesText(const EditorGithub::SReleaseInfo& r)
{
	const u32 n = (u32)r.assets.size();
	return NqUtil::Format("%u %s \xC2\xB7 %s", n, n == 1 ? "file" : "files", SizeText(r.bytes).c_str());
}

// GitHub gives the time in UTC; the author reads it in their own
xr_string LocalTime(const xr_string& utc)
{
	SYSTEMTIME st = {}, local = {};
	if (5 != sscanf_s(utc.c_str(), "%hu-%hu-%hu %hu:%hu", &st.wYear, &st.wMonth, &st.wDay, &st.wHour, &st.wMinute) ||
		!::SystemTimeToTzSpecificLocalTime(nullptr, &st, &local))
		return utc;
	return NqUtil::Format("%04u-%02u-%02u %02u:%02u", local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute);
}

// the order of the State column: what players get first
int StateRank(const EditorGithub::SReleaseInfo& r)
{
	return r.latest ? 0 : r.draft ? 3 : r.prerelease ? 2 : 1;
}

void StateBadge(const EditorGithub::SReleaseInfo& r)
{
	if (r.latest)			XUI::Badge("Latest", XUI::Tone::Success);
	else if (r.draft)		XUI::Badge("Draft", XUI::Tone::Muted);
	else if (r.prerelease)	XUI::Badge("Pre-release", XUI::Tone::Info);
}

// the State cell: a badge for what stands out, plain text for a release that is simply out
void StateCell(const EditorGithub::SReleaseInfo& r)
{
	if (r.latest || r.draft || r.prerelease)
	{
		StateBadge(r);
		return;
	}
	ImGui::PushStyleColor(ImGuiCol_Text, Token(XFinedTheme::ColorToken::TextSecondary));
	XUI::TextEllipsis("Published");
	ImGui::PopStyleColor();
}

// reads the list again. A failure is what the tab shows - never a window of its own
void RefreshReleases()
{
	ManageReap();
	s_ListedAt = ::GetTickCount();
	s_SetupGen = EditorMod::ManifestGeneration();
	if (ManageRunning() || JobRunning()) return;
	xr_string err;
	EReleasesSetup setup = rsOk;
	const bool started = ManageStart(SManageRequest(), err, &setup);
	s_ReleasesSetup = setup;
	if (!started) SetManageError(err);
}

void OpenEdit(const EditorGithub::SReleaseInfo& target)
{
	// one modal at a time: two dialogs that open themselves close each other every frame
	if (s_WantDialog || s_WantProgress) return;
	s_EditTarget = target;
	strncpy_s(s_EditTitle, sizeof(s_EditTitle), target.title.c_str(), _TRUNCATE);
	s_EditNotes = target.notes;
	s_WantEdit = true;
}

// The one question before a release goes: the release, its files and its tag by name, and what players get
// instead when it is the latest one. GitHub keeps no copy.
void AskDelete(const SManageView& view, const EditorGithub::SReleaseInfo& target)
{
	// the list is newest first: the first other release players may receive is the fallback
	xr_string next;
	for (u32 i = 0; i < view.releases.size() && next.empty(); ++i)
	{
		const EditorGithub::SReleaseInfo& r = view.releases[i];
		if (r.id != target.id && !r.draft && !r.prerelease) next = r.tag;
	}
	const xr_string tag = target.tag.empty() ? xr_string("(no tag)") : target.tag;
	const xr_string headline = NqUtil::Format("Delete %s %s \"%s\" from %s?", target.draft ? "draft release" : "release", tag.c_str(),
		target.title.c_str(), view.repo.c_str());
	xr_string consequence;
	if (target.latest)
		consequence = next.empty() ? xr_string("It is the latest release: players are offered no update until another one is published. ") :
			"It is the latest release: GitHub then serves " + next + " to players. ";
	if (!target.draft && !target.prerelease)
		consequence += "Players who have this version keep it; published again, the game offers it to them as a fix. ";
	consequence += "Deleted from GitHub for good - this cannot be undone.";

	XUI::ConfirmSpec spec;
	spec.id				= "delete_release";
	spec.title			= "Delete Release";
	spec.headline		= headline.c_str();
	spec.items			= target.assets;
	if (!target.draft && !target.tag.empty()) spec.items.push_back("the git tag " + target.tag);
	spec.consequence	= consequence.c_str();
	spec.action			= "Delete Release";
	spec.destructive	= true;
	const u64 id = target.id;
	XUI::Confirm(spec, [id](XUI::Answer answer)
	{
		if (XUI::Answer::Yes != answer) return;
		SManageRequest rq;
		rq.op = moDelete;
		rq.id = id;
		xr_string err;
		if (!ManageStart(rq, err)) SetManageError(err);
		if (s_SelectedId == id) s_SelectedId = 0;
	});
}

// the order the table's header asks for; GitHub's own (newest first, drafts on top) is the published time falling
void SortedOrder(const SManageView& view, xr_vector<u32>& order)
{
	order.resize(view.releases.size());
	for (u32 i = 0; i < order.size(); ++i) order[i] = i;
	const ImGuiTableSortSpecs* specs = ImGui::TableGetSortSpecs();
	if (!specs || !specs->SpecsCount) return;
	const int column = specs->Specs[0].ColumnIndex;
	const bool descending = ImGuiSortDirection_Descending == specs->Specs[0].SortDirection;
	std::stable_sort(order.begin(), order.end(), [&](u32 a, u32 b)
	{
		const EditorGithub::SReleaseInfo& x = view.releases[a];
		const EditorGithub::SReleaseInfo& y = view.releases[b];
		int c = 0;
		switch (column)
		{
		case 0:	c = _stricmp(x.tag.c_str(), y.tag.c_str()); break;
		case 1:	c = _stricmp(x.title.c_str(), y.title.c_str()); break;
		case 2:	c = StateRank(x) - StateRank(y); break;
		// a draft is not published yet: it is the newest
		case 3:	c = strcmp(x.published.empty() ? "~" : x.published.c_str(), y.published.empty() ? "~" : y.published.c_str()); break;
		default: c = x.bytes < y.bytes ? -1 : x.bytes > y.bytes ? 1 : 0; break;
		}
		return descending ? c > 0 : c < 0;
	});
}

void RowMenu(const SManageView& view, const EditorGithub::SReleaseInfo& r, bool busy)
{
	if (ImGui::MenuItem("Edit Title and Notes...", nullptr, false, !busy))	OpenEdit(r);
	if (ImGui::MenuItem("Open Release Page", nullptr, false, IsGithubPage(r.url)))	OpenGithubPage(r.url);
	if (ImGui::MenuItem("Copy Tag", nullptr, false, !r.tag.empty()))	ImGui::SetClipboardText(r.tag.c_str());
	ImGui::Separator();
	const bool drop = XUI::MenuItem("Delete Release...", XUI::Icon::None, nullptr, !busy, true);
	if (drop) AskDelete(view, r);
}

void DrawReleasesTable(const SManageView& view, float width, float height, bool busy)
{
	// the column lines stay in the header - under the last row is empty table, not columns (#33) - and show in full while
	// a column is resized
	const ImGuiTableFlags flags = ImGuiTableFlags_Sortable | ImGuiTableFlags_ScrollY | ImGuiTableFlags_RowBg | ImGuiTableFlags_BordersOuter |
		ImGuiTableFlags_Resizable | ImGuiTableFlags_NoBordersInBodyUntilResize | ImGuiTableFlags_SizingStretchProp | ImGuiTableFlags_PadOuterX;
	// Tag is as wide as the widest tag (a long one ellipsizes), Title takes every change of the width
	float tag_w = XUI::TextWidth(XUI::Font::Small, "Tag") + XUI::Em(1.25f);
	for (const EditorGithub::SReleaseInfo& r : view.releases)
		tag_w = _max(tag_w, XUI::TextWidth(XUI::Font::Body, r.tag.empty() ? "(no tag)" : r.tag.c_str()) + XUI::Pt(2.f));
	tag_w = _min(tag_w, XUI::Em(14.f));
	ImGui::PushStyleColor(ImGuiCol_ChildBg, Token(XFinedTheme::ColorToken::Inset));
	const bool open = ImGui::BeginTable("##releases", 5, flags, ImVec2(width, height));
	ImGui::PopStyleColor();
	if (!open) return;
	ImGui::TableSetupScrollFreeze(0, 1);
	ImGui::TableSetupColumn("Tag", ImGuiTableColumnFlags_WidthFixed | ImGuiTableColumnFlags_NoResize, tag_w);
	ImGui::TableSetupColumn("Title", ImGuiTableColumnFlags_WidthStretch, 1.f);
	ImGui::TableSetupColumn("State", ImGuiTableColumnFlags_WidthFixed, XUI::Em(6.f));
	ImGui::TableSetupColumn("Published (local)", ImGuiTableColumnFlags_WidthFixed | ImGuiTableColumnFlags_DefaultSort |
		ImGuiTableColumnFlags_PreferSortDescending, XUI::Em(9.f));
	ImGui::TableSetupColumn("Files", ImGuiTableColumnFlags_WidthFixed, XUI::Em(8.5f));
	XUI::TableHeaders();
	xr_vector<u32> order;
	SortedOrder(view, order);
	for (u32 k : order)
	{
		const EditorGithub::SReleaseInfo& r = view.releases[k];
		ImGui::TableNextRow();
		ImGui::PushID(NqUtil::Format("%llu", r.id).c_str());
		ImGui::TableNextColumn();
		const ImVec2 at = ImGui::GetCursorScreenPos();
		if (ImGui::Selectable("##row", r.id == s_SelectedId, ImGuiSelectableFlags_SpanAllColumns | ImGuiSelectableFlags_AllowOverlap))
			s_SelectedId = r.id;
		if (ImGui::BeginPopupContextItem("##row_menu"))
		{
			s_SelectedId = r.id;
			RowMenu(view, r, busy);
			ImGui::EndPopup();
		}
		ImGui::SetCursorScreenPos(at);
		XUI::TextEllipsis(r.tag.empty() ? "(no tag)" : r.tag.c_str());
		ImGui::TableNextColumn();
		XUI::TextEllipsis(r.title.c_str());
		ImGui::TableNextColumn();
		StateCell(r);
		ImGui::TableNextColumn();
		XUI::TextEllipsis(r.published.empty() ? "Not published" : LocalTime(r.published).c_str());
		ImGui::TableNextColumn();
		XUI::TextEllipsis(FilesText(r).c_str());
		ImGui::PopID();
	}
	ImGui::EndTable();
}

void DrawReleaseDetails(const SManageView& view, bool busy)
{
	const EditorGithub::SReleaseInfo* r = Selected(view);
	if (!r)
	{
		XUI::EmptyState(XUI::Icon::Releases, "No release selected", "Select a release to see its files and notes.");
		return;
	}
	{
		XUI::Toolbar tb("release_actions", XUI::ToolbarPanel);
		tb.Group(2);
		if (tb.Action("edit", XUI::Icon::Rename, "Edit...", busy ? "A release job is running: wait for it to finish." :
				"Edit the title and the notes. The files, the tag and the latest mark stay as they are.", !busy))
			OpenEdit(*r);
		tb.Group(1);
		if (tb.Action("page", XUI::Icon::External, "Open Page", "The release's page on GitHub.", IsGithubPage(r->url)))
			OpenGithubPage(r->url);
		tb.Spring();
		tb.Group(3);
		const bool drop = tb.Action("delete", XUI::Icon::Delete, "Delete...", busy ? "A release job is running: wait for it to finish." :
			"Delete the release and its git tag from GitHub. Asks first, naming what goes.", !busy, false, XUI::Kind::GhostDanger);
		if (drop) AskDelete(view, *r);
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
	{
		XUI::FontScope title(XUI::Font::Title);
		XUI::TextEllipsis(r->tag.empty() ? "(no tag)" : r->tag.c_str(), _max(XUI::Em(4.f), ImGui::GetContentRegionAvail().x - XUI::Em(6.f)));
	}
	if (r->latest || r->draft || r->prerelease)
	{
		ImGui::SameLine();
		StateBadge(*r);
	}
	if (!r->title.empty())
	{
		ImGui::PushTextWrapPos(0.f);
		ImGui::TextUnformatted(r->title.c_str());
		ImGui::PopTextWrapPos();
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
	if (XUI::BeginPropertyGrid("release", .35f))
	{
		if (XUI::PropertyRow("Published", "In your local time. A draft is not published: players never see it.", XUI::PropReadOnly))
			XUI::PropertyText(r->published.empty() ? "Not published (draft)" : LocalTime(r->published).c_str());
		if (XUI::PropertyRow("Files", "What the release holds: the packages, the file index and the descriptor.", XUI::PropReadOnly))
			XUI::PropertyText(FilesText(*r).c_str());
		XUI::EndPropertyGrid();
	}
	ImGui::PushStyleColor(ImGuiCol_Text, Token(XFinedTheme::ColorToken::TextSecondary));
	ImGui::Indent(XUI::Pt(6.f));
	for (const xr_string& name : r->assets)
		XUI::TextEllipsis(name.c_str(), -1.f, XUI::Clip::Middle);
	ImGui::Unindent(XUI::Pt(6.f));
	ImGui::PopStyleColor();
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
	if (XUI::Section("Notes", true))
	{
		if (r->notes.empty())
		{
			ImGui::PushStyleColor(ImGuiCol_Text, Token(XFinedTheme::ColorToken::TextMuted));
			ImGui::TextUnformatted("No notes.");
			ImGui::PopStyleColor();
		}
		else
		{
			ImGui::PushTextWrapPos(0.f);
			ImGui::TextUnformatted(r->notes.c_str());
			ImGui::PopTextWrapPos();
		}
		XUI::EndSection();
	}
}

void DrawReleasesBody()
{
	ManageReap();
	const SManageView& view = ShownView();
	const bool busy = ReleasesBusy(view);
	if (!EditorProject::Active() || rsNoProject == s_ReleasesSetup)
	{
		XUI::EmptyState(XUI::Icon::Releases, "No project", "Open a project to see the releases of its module.");
		return;
	}
	// Mod Settings set the repository meanwhile: the list follows
	if (rsNoRepo == s_ReleasesSetup && s_SetupGen != EditorMod::ManifestGeneration())
		RefreshReleases();
	if (rsNoRepo == s_ReleasesSetup)
	{
		if (XUI::EmptyState(XUI::Icon::Releases, "No GitHub repository", "The game updates this module from the latest release of the "
				"GitHub repository Mod Settings names.", "Open Mod Settings"))
			EditorMod::RequestEditManifest();
		return;
	}
	if (rsNoAccount == s_ReleasesSetup)
	{
		if (XUI::EmptyState(XUI::Icon::Link, "Not signed in to GitHub", "Sign in from Publish Release, sign in to the GitHub CLI "
				"(gh auth login) or set GH_TOKEN, then refresh.", "Sign In..."))
			EditorPublish::RequestPublish();
		return;
	}

	if (JobRunning())
		XUI::Banner(XUI::Tone::Info, "A release is being published: the list is read again when it is out.");
	if (msRunning == view.state)
		XUI::BusyLine(moDelete == view.op ? "Deleting the release..." : moEdit == view.op ? "Saving the release..." : "Reading the releases...");
	else if (!view.error.empty() && 1 == XUI::Banner(XUI::Tone::Error, view.error_text.c_str(), "Retry"))
		RefreshReleases();
	if (!view.note.empty())
		XUI::Banner(view.tag_kept ? XUI::Tone::Warning : XUI::Tone::Success, view.note.c_str());
	if (!view.listed)
		return;
	if (view.releases.empty())
	{
		if (XUI::EmptyState(XUI::Icon::Releases, "No releases yet", "Publish Release packages the module and publishes it here.",
				"Publish Release..."))
			EditorPublish::RequestPublish();
		return;
	}

	// the table takes what the details leave; too narrow for both, the table alone
	const ImVec2 avail = ImGui::GetContentRegionAvail();
	const float gap = ImGui::GetStyle().ItemSpacing.x;
	if (avail.x < XUI::Pt(400.f + 260.f) + gap)
	{
		DrawReleasesTable(view, avail.x, avail.y, busy);
		return;
	}
	float side_pt = 340.f;
	XUI::SplitterTail("releases.details", true, &side_pt, 400.f, 260.f, 340.f);
	const float side = XUI::Pt(side_pt);
	DrawReleasesTable(view, avail.x - side - gap, avail.y, busy);
	ImGui::SameLine(0.f, gap);
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(12.f), XUI::Pt(8.f)));
	ImGui::PushStyleColor(ImGuiCol_Border, Token(XFinedTheme::ColorToken::BorderSubtle));
	const bool shown = ImGui::BeginChild("##release_details", ImVec2(side, avail.y), ImGuiChildFlags_Borders | ImGuiChildFlags_AlwaysUseWindowPadding);
	ImGui::PopStyleColor();
	ImGui::PopStyleVar();
	if (shown)
		DrawReleaseDetails(view, busy);
	ImGui::EndChild();
}

void ReleasesToolbar(XUI::Toolbar& tb)
{
	const SManageView& view = ShownView();
	const bool busy = ReleasesBusy(view);
	tb.Group(2);
	if (tb.Action("refresh", XUI::Icon::Refresh, "Refresh", busy ? "A release job is running: wait for it to finish." :
			"Read the releases from GitHub again (F5).", !busy))
		RefreshReleases();
	tb.Group(1);
	const xr_string page = view.repo.empty() ? xr_string() : EditorGithub::ReleasesPage(view.repo.c_str());
	if (tb.Action("github", XUI::Icon::External, "Open on GitHub", "The repository's releases page in the browser.", IsGithubPage(page)))
		OpenGithubPage(page);
}

// the centre of the status bar: the repository and what it holds
void ReleasesStatus()
{
	const SManageView& view = ShownView();
	if (view.repo.empty()) return;
	xr_string text = view.repo;
	if (view.listed)
	{
		u32 drafts = 0;
		for (const EditorGithub::SReleaseInfo& r : view.releases)
			drafts += r.draft ? 1 : 0;
		const u32 out = (u32)view.releases.size() - drafts;
		text += NqUtil::Format(" \xC2\xB7 %u published", out);
		if (drafts) text += NqUtil::Format(" \xC2\xB7 %u %s", drafts, drafts == 1 ? "draft" : "drafts");
	}
	ImGui::PushStyleColor(ImGuiCol_Text, Token(XFinedTheme::ColorToken::TextSecondary));
	ImGui::TextUnformatted(text.c_str());
	ImGui::PopStyleColor();
}

// Releases is one of the editor's major tabs (XUI::Shell): a singleton without a file of its own
void ShowReleasesTab()
{
	s_ReleasesOpen = true;
	if (XUI::Shell::IsOpen(kReleasesTab))
	{
		XUI::Shell::Activate(kReleasesTab);
		return;
	}
	XUI::Shell::DocDesc d;
	d.id		= kReleasesTab;
	d.kind		= kReleasesTab;
	d.icon		= XUI::Icon::Releases;
	d.label		= [] { return xr_string("Releases"); };
	d.tooltip	= []
	{
		const SManageView& view = ShownView();
		return view.repo.empty() ? xr_string("The module's releases on GitHub") : "The releases of " + view.repo + " on GitHub";
	};
	d.alive		= [] { return s_ReleasesOpen; };
	d.toolbar	= [](XUI::Toolbar& tb) { ReleasesToolbar(tb); };
	d.body		= [] { DrawReleasesBody(); };
	d.status	= [] { ReleasesStatus(); };
	d.shortcut	= [](int key, int mods)
	{
		if (mods) return false;
		if (ImGuiKey_F5 == key)
		{
			RefreshReleases();
			return true;
		}
		const SManageView& view = ShownView();
		const EditorGithub::SReleaseInfo* r = Selected(view);
		if (ImGuiKey_Delete != key || !r || ReleasesBusy(view)) return false;
		AskDelete(view, *r);
		return true;
	};
	d.edit		= [](LPCSTR action, bool run)
	{
		if (0 != xr_strcmp(action, "delete")) return false;
		const SManageView& view = ShownView();
		const EditorGithub::SReleaseInfo* r = Selected(view);
		const bool can = r && !ReleasesBusy(view);
		if (run && can) AskDelete(view, *r);
		return can;
	};
	d.on_activate = []
	{
		if (!ManageRunning() && ::GetTickCount() - s_ListedAt > kRelistMs) RefreshReleases();
	};
	d.on_close	= [] { s_ReleasesOpen = false; };
	d.restore	= [] { return xr_string(); };
	XUI::Shell::Open(d, true);
}

// ImGui writes into the string's own buffer and asks here when it runs out of room
int GrowNotes(ImGuiInputTextCallbackData* data)
{
	if (ImGuiInputTextFlags_CallbackResize == data->EventFlag)
	{
		xr_string* text = (xr_string*)data->UserData;
		text->resize((size_t)data->BufTextLen);
		data->Buf = &(*text)[0];
	}
	return 0;
}

void DrawEditRelease()
{
	if (!XUI::BeginDialog(kTitleEdit, &s_WantEdit, XUI::DialogSize::M))
		return;
	const EditorGithub::SReleaseInfo& r = s_EditTarget;
	const bool too_long = s_EditNotes.size() > kMaxNotes, title_long = xr_strlen(s_EditTitle) > 200;
	const bool busy = ManageRunning() || JobRunning();
	XUI::BeginDialogBody();
	{
		XUI::FontScope strong(XUI::Font::Strong);
		XUI::TextEllipsis(NqUtil::Format("%s %s", r.draft ? "Draft release" : "Release", r.tag.empty() ? "(no tag)" : r.tag.c_str()).c_str());
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
	FieldLabel("Title");
	ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
	ImGui::InputText("##title", s_EditTitle, sizeof(s_EditTitle));
	FieldLabel("Notes");
	ImGui::InputTextMultiline("##notes", &s_EditNotes[0], s_EditNotes.capacity() + 1, ImVec2(ImGui::GetContentRegionAvail().x, XUI::Em(12.f)),
		ImGuiInputTextFlags_CallbackResize, &GrowNotes, &s_EditNotes);
	XUI::Tip("What changed, shown on the release page. Markdown.");
	XUI::StatusText(XUI::Tone::Muted, "The files, the tag and the latest mark of the release stay as they are.");
	if (too_long)
		XUI::StatusText(XUI::Tone::Warning, "GitHub takes notes of 120 000 characters at most.");
	if (title_long)
		XUI::StatusText(XUI::Tone::Warning, "GitHub takes a title of 200 characters at most.");
	if (XUI::BeginDialogFooter())
	{
		LPCSTR why = too_long ? "The notes are longer than GitHub takes." : title_long ? "The title is longer than GitHub takes." :
			"A release job is running: wait for it to finish.";
		if (XUI::DialogButton("Save", XUI::Kind::Primary, true, !too_long && !title_long && !busy, why))
		{
			SManageRequest rq;
			rq.op = moEdit;
			rq.id = r.id;
			rq.has_title = rq.has_notes = true;
			rq.title = s_EditTitle;
			rq.notes = s_EditNotes.c_str();	// up to the terminator: the buffer may run past the text
			xr_string err;
			if (!ManageStart(rq, err)) SetManageError(err);
			s_WantEdit = false;
		}
		if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
			s_WantEdit = false;
	}
	XUI::EndDialog();
}

void ReleasesJson(xr_string& out)
{
	ManageReap();
	const SManageView view = ManageSnapshot();
	static const char* names[] = { "idle", "running", "done", "failed" };
	out = msFailed == view.state ? "{\"ok\":false" : "{\"ok\":true";
	JsonField(out, "state", names[view.state]);
	out += msRunning == view.state ? ",\"pending\":true" : ",\"pending\":false";
	JsonField(out, "repo", view.repo.c_str());
	JsonField(out, "note", view.note.c_str());
	out += view.tag_kept ? ",\"tag_kept\":true" : ",\"tag_kept\":false";
	JsonField(out, "error", view.error.c_str());
	// what the Releases tab shows: whether it is there, and the release its details are about
	out += NqUtil::Format(",\"tab\":%s,\"selected\":%llu", s_ReleasesOpen ? "true" : "false", s_SelectedId);
	out += view.listed ? ",\"listed\":true,\"releases\":[" : ",\"listed\":false,\"releases\":[";
	for (u32 i = 0; i < view.releases.size(); ++i)
	{
		const EditorGithub::SReleaseInfo& r = view.releases[i];
		out += NqUtil::Format("%s{\"id\":%llu", i ? "," : "", r.id);
		JsonField(out, "tag", r.tag.c_str());
		JsonField(out, "title", r.title.c_str());
		JsonField(out, "notes", r.notes.c_str());
		out += NqUtil::Format(",\"draft\":%s,\"prerelease\":%s,\"latest\":%s", r.draft ? "true" : "false",
			r.prerelease ? "true" : "false", r.latest ? "true" : "false");
		JsonField(out, "published", r.published.c_str());
		JsonField(out, "url", r.url.c_str());
		out += NqUtil::Format(",\"bytes\":%llu,\"assets\":[", r.bytes);
		for (u32 a = 0; a < r.assets.size(); ++a)
		{
			out += a ? ",\"" : "\"";
			JsonAppend(out, r.assets[a].c_str());
			out += "\"";
		}
		out += "]}";
	}
	out += "]}";
}

// "id" as a bare number or a quoted one
u64 ArgU64(LPCSTR raw, LPCSTR field)
{
	char pat[64];
	sprintf_s(pat, "\"%s\"", field);
	const char* k = raw ? strstr(raw, pat) : 0;
	if (!k) return 0;
	const char* c = strchr(k + xr_strlen(pat), ':');
	if (!c) return 0;
	do ++c; while (*c == ' ' || *c == '"');
	u64 value = 0;
	for (; *c >= '0' && *c <= '9'; ++c) value = value * 10 + (u64)(*c - '0');
	return value;
}

void StatusJson(xr_string& out)
{
	Reap();
	const SJobView job = JobSnapshot();
	const bool failed = jsFailed == job.state || jsCancelled == job.state;
	out = failed ? "{\"ok\":false" : "{\"ok\":true";
	JsonField(out, "state", StateText(job.state));
	out += Running(job.state) ? ",\"pending\":true" : ",\"pending\":false";
	JsonField(out, "id", job.id.c_str());
	JsonField(out, "version", job.version.c_str());
	JsonField(out, "repo", job.repo.c_str());
	JsonField(out, "tag", job.tag.c_str());
	JsonField(out, "stage", job.stage.c_str());
	out += NqUtil::Format(",\"sent\":%llu,\"total\":%llu", job.sent, job.total);
	JsonField(out, "url", job.url.c_str());
	out += job.served ? ",\"served\":true" : ",\"served\":false";
	JsonField(out, "warnings", job.warnings.c_str());
	JsonField(out, "error", jsCancelled == job.state ? "cancelled" : job.error.c_str());
	out += ",\"virustotal\":";
	EditorVirusTotal::Json(out, job.virustotal);
	out += "}";
}

// the source alone: a token goes in through MCP (save_token) and never comes out
void AccountJson(xr_string& out)
{
	EditorGithub::SAccount account;
	EditorGithub::ResolveAccount(account);
	out = EditorGithub::asNone == account.source ? "{\"ok\":true,\"signed_in\":false" : "{\"ok\":true,\"signed_in\":true";
	JsonField(out, "source", EditorGithub::SourceText(account.source));
	out += EditorGithub::DeviceSignInAvailable() ? ",\"device_sign_in\":true}" : ",\"device_sign_in\":false}";
}

// The caller gets the code, it is confirmed on GitHub, and the token goes from
// GitHub straight into the Credential Manager.
void SignJson(xr_string& out)
{
	const SSignView sign = SignSnapshot();
	static const char* names[] = { "idle", "starting", "waiting", "done", "failed" };
	out = ssFailed == sign.state ? "{\"ok\":false" : "{\"ok\":true";
	JsonField(out, "state", names[sign.state]);
	out += (ssStarting == sign.state || ssWaiting == sign.state) ? ",\"pending\":true" : ",\"pending\":false";
	JsonField(out, "user_code", ssWaiting == sign.state ? sign.user_code.c_str() : "");
	JsonField(out, "verification_uri", ssWaiting == sign.state ? sign.uri.c_str() : "");
	JsonField(out, "error", sign.error.c_str());
	out += "}";
	if (ssDone == sign.state || ssFailed == sign.state)
	{
		// reported once; the dialog re-reads the account when it opens
		if (s_SignThread.joinable()) s_SignThread.join();
		SignSet(ssIdle);
	}
}
} // namespace

//------------------------------------------------------------------------------
// public
//------------------------------------------------------------------------------
bool EditorPublish::CanPublish()
{
	return EditorProject::Active() && !s_PublishQueued;
}

void EditorPublish::ButtonText(char* dst, u32 size)
{
	const SJobView job = JobSnapshot();
	if (jsScanning == job.state)
		strcpy_s(dst, size, "Scanning Release");
	else if (jsUploading == job.state)
		sprintf_s(dst, size, "Publishing %u%%", job.total ? (u32)(job.sent * 100 / job.total) : 0u);
	else
		strcpy_s(dst, size, "Publish Release");
}

void EditorPublish::RequestPublish()
{
	// one modal at a time: two dialogs that open themselves close each other every frame
	if (!CanPublish() || s_WantEdit) return;
	Reap();
	if (JobRunning()) { s_WantProgress = true; return; }

	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m))
	{
		XUI::Notice(XUI::Tone::Error, "Publish Release", "Cannot publish the release: mod.ltx is not in the project.");
		return;
	}
	ReadManifestFields();
	xr_string game_version;
	EditorMod::LinkedGameVersion(game_version);
	s_Requires = game_version;
	s_Notes[0]		= 0;
	s_TagAuto		= true;
	s_TagFor.clear();
	s_TitleAuto		= true;
	s_Prerelease	= false;
	s_Latest		= true;
	s_ReplacedFor.clear();
	s_AccountError.clear();
	RefreshAccount();
	// the releases are read again for every open - one may have gone out from elsewhere meanwhile - and from here, so
	// that an MCP caller that opened the dialog can follow the read with mod_releases status
	s_ListAskedFor.clear();
	EnsureReleaseList();
	s_WantDialog	= true;
}

void EditorPublish::RegisterDocuments()
{
	// once: a factory replaced while the shell runs it would pull the ground from under it
	static bool s_Registered = false;
	if (s_Registered) return;
	s_Registered = true;
	XUI::Shell::RegisterFactory(kReleasesTab, [](LPCSTR)
	{
		EditorPublish::RequestReleases();
		return XUI::Shell::IsOpen(kReleasesTab);
	});
}

void EditorPublish::RequestReleases()
{
	if (!EditorProject::Active()) return;
	RegisterDocuments();
	Reap();
	ShowReleasesTab();
	RefreshReleases();
}

void EditorPublish::DrawUI()
{
	Reap();
	ManageReap();
	if (!EditorProject::Active()) return;
	if (s_RefreshWanted && !JobRunning() && !ManageRunning())
	{
		s_RefreshWanted = false;
		if (s_ReleasesOpen) RefreshReleases();
	}
	DrawDialog();
	DrawProgress();
	DrawEditRelease();
	if (s_Result.want)
	{
		const SPendingResult r = s_Result;
		s_Result = SPendingResult();
		const xr_string url = r.url;
		XUI::Result("publish_result", "Publish Release", r.tone, r.headline.c_str(), r.rows, XUI::Strings(), r.warnings,
			IsGithubPage(url) ? "Open Release Page" : nullptr, [url] { OpenGithubPage(url); });
	}
}

void EditorPublish::Shutdown()
{
	s_JobCancel.Cancel();
	if (s_JobThread.joinable()) s_JobThread.join();
	s_SignCancel.Cancel();
	if (s_SignThread.joinable()) s_SignThread.join();
	s_ManageCancel.Cancel();
	if (s_ManageThread.joinable()) s_ManageThread.join();
	EditorGithub::WipeString(s_Account.token);
}

void EditorPublish::McpReleases(LPCSTR raw, xr_string& out)
{
	char action[32] = {};
	if (!XFinedMCP::GetArg(raw, "action", action, sizeof(action))) strcpy_s(action, "list");

	if (0 == xr_strcmp(action, "status"))	{ ReleasesJson(out); return; }
	if (0 == xr_strcmp(action, "cancel"))
	{
		if (ManageRunning()) s_ManageCancel.Cancel();
		ReleasesJson(out);
		return;
	}
	const bool select = 0 == xr_strcmp(action, "select");
	SManageRequest rq;
	rq.op = 0 == xr_strcmp(action, "delete") ? moDelete : 0 == xr_strcmp(action, "edit") ? moEdit : moList;
	if (moList == rq.op && !select && 0 != xr_strcmp(action, "list"))
		{ out = "{\"ok\":false,\"error\":\"unknown action: list, edit, delete, select, status or cancel expected\"}"; return; }

	// MCP is the programmatic path: the caller has decided, nothing is asked
	if (moList != rq.op || select)
	{
		xr_vector<char> arg(xr_strlen(raw) + 1);
		if (XFinedMCP::GetArg(raw, "tag", arg.data(), (u32)arg.size())) rq.tag = arg.data();
		rq.id = ArgU64(raw, "id");
		if (!rq.id && rq.tag.empty())
			{ out = "{\"ok\":false,\"error\":\"name the release: 'id' or 'tag'\"}"; return; }
		if (moEdit == rq.op)
		{
			// a field that is absent keeps its value; an empty one clears it
			if ((rq.has_title = XFinedMCP::GetArg(raw, "title", arg.data(), (u32)arg.size())))	rq.title = arg.data();
			if ((rq.has_notes = XFinedMCP::GetArg(raw, "notes", arg.data(), (u32)arg.size())))	rq.notes = arg.data();
			if (!rq.has_title && !rq.has_notes)
				{ out = "{\"ok\":false,\"error\":\"edit needs 'title' or 'notes'\"}"; return; }
			if (rq.title.size() > 200 || rq.notes.size() > kMaxNotes)
				{ out = "{\"ok\":false,\"error\":\"the title takes 200 characters, the notes 120 000\"}"; return; }
		}
	}
	// the release the tab's details are about, from the list as it was last read: nothing goes to GitHub
	if (select)
	{
		const SManageView view = ManageSnapshot();
		const EditorGithub::SReleaseInfo* found = nullptr;
		u32 matches = 0;
		for (const EditorGithub::SReleaseInfo& r : view.releases)
			if (rq.id ? r.id == rq.id : r.tag == rq.tag) { found = &r; ++matches; }
		// show=edit puts the edit dialog up for that release, to be looked at: only a click in it saves anything
		char show[16] = {};
		XFinedMCP::GetArg(raw, "show", show, sizeof(show));
		xr_string err;
		if (show[0] && 0 != xr_strcmp(show, "edit"))	err = "show: edit expected";
		else if (!view.listed)							err = "the list is not read yet: mod_releases action=list first";
		else if (!matches)								err = rq.id ? NqUtil::Format("no release has the id %llu", rq.id) : "no release carries the tag " + rq.tag;
		else if (matches > 1)							err = NqUtil::Format("%u releases carry the tag %s - name one by its id", matches, rq.tag.c_str());
		else if (show[0] && (s_WantDialog || s_WantProgress))	err = "the Publish Release dialog is up: one release dialog at a time";
		if (!err.empty())
		{
			out = "{\"ok\":false,\"error\":\"";
			JsonAppend(out, err.c_str());
			out += "\"}";
			return;
		}
		s_SelectedId = found->id;
		if (show[0]) OpenEdit(*found);
		ReleasesJson(out);
		return;
	}
	xr_string err;
	if (!ManageStart(rq, err))
	{
		out = "{\"ok\":false,\"error\":\"";
		JsonAppend(out, err.c_str());
		out += "\"}";
		return;
	}
	ReleasesJson(out);
}

void EditorPublish::McpPublish(LPCSTR raw, xr_string& out)
{
	char action[32] = {};
	if (!XFinedMCP::GetArg(raw, "action", action, sizeof(action))) strcpy_s(action, "start");

	if (0 == xr_strcmp(action, "status"))	{ StatusJson(out); return; }
	if (0 == xr_strcmp(action, "cancel"))
	{
		if (JobRunning()) s_JobCancel.Cancel();
		StatusJson(out);
		return;
	}
	if (0 == xr_strcmp(action, "account"))			{ AccountJson(out); return; }
	if (0 == xr_strcmp(action, "sign_in_status"))	{ SignJson(out); return; }
	if (0 == xr_strcmp(action, "sign_in"))
	{
		if (!EditorGithub::DeviceSignInAvailable())
		{
			out = "{\"ok\":false,\"error\":\"the device sign-in is not set up in this build: use a token, GH_TOKEN or the GitHub CLI\"}";
			return;
		}
		const ESignState now = SignSnapshot().state;
		if (ssStarting != now && ssWaiting != now)
		{
			SignStart();
			// the caller hands the code over; no clipboard, no browser behind its back
			s_SignShown = true;
		}
		SignJson(out);
		return;
	}
	if (0 == xr_strcmp(action, "save_token"))
	{
		// The one request that carries a token. It goes into the Credential Manager
		// and nowhere else: not into the answer, not into the log.
		xr_vector<char> arg(xr_strlen(raw) + 1);
		xr_string err;
		const bool given = XFinedMCP::GetArg(raw, "token", arg.data(), (u32)arg.size());
		const bool saved = given && EditorGithub::SaveToken(arg.data(), err);
		SecureZeroMemory(arg.data(), arg.size());
		if (!saved)
		{
			out = "{\"ok\":false,\"error\":\"";
			JsonAppend(out, given ? err.c_str() : "save_token needs 'token'");
			out += "\"}";
			return;
		}
		AccountJson(out);
		return;
	}
	if (0 == xr_strcmp(action, "sign_out"))
	{
		SignStop();
		EditorGithub::ForgetToken();
		AccountJson(out);
		return;
	}
	if (0 != xr_strcmp(action, "start"))
	{
		out = "{\"ok\":false,\"error\":\"unknown action: start, status, cancel, account, save_token, sign_in, sign_in_status or sign_out expected\"}";
		return;
	}

	// request-sized, so over-long notes reach the check instead of reading as absent
	xr_vector<char> arg(xr_strlen(raw) + 1);
	// the author key's password, when the key file has one; never echoed or logged
	if (XFinedMCP::GetArg(raw, "password", arg.data(), (u32)arg.size()) && arg[0])
	{
		xr_string key_err;
		const bool opened = EditorCatalog::Unlock(arg.data(), key_err);
		SecureZeroMemory(arg.data(), arg.size());
		if (!opened)
		{
			out = "{\"ok\":false,\"error\":\"author key: ";
			JsonAppend(out, key_err.c_str());
			out += "\"}";
			return;
		}
	}
	SOptions options;
	if (XFinedMCP::GetArg(raw, "tag", arg.data(), (u32)arg.size()))		options.tag = arg.data();
	if (XFinedMCP::GetArg(raw, "title", arg.data(), (u32)arg.size()))	options.title = arg.data();
	if (XFinedMCP::GetArg(raw, "notes", arg.data(), (u32)arg.size()))	options.notes = arg.data();
	// absent = the linked game's version, present but empty = no requirement
	options.requires_given	= XFinedMCP::GetArg(raw, "requires_game", arg.data(), (u32)arg.size());
	if (options.requires_given) options.requires_game = arg.data();
	options.prerelease		= ArgBool(raw, "prerelease", false);
	options.latest			= ArgBool(raw, "latest", true);

	SStarted started;
	xr_string err;
	if (!Start(options, false, started, err))
	{
		out = "{\"ok\":false,\"error\":\"";
		JsonAppend(out, err.c_str());
		out += "\"}";
		return;
	}
	StatusJson(out);
	// the packaging half of the answer, as mod_package gives it
	out.erase(out.size() - 1);
	out += ",\"assets\":[";
	const EditorMod::SPackageResult& res = started.package;
	xr_vector<EditorMod::SPackageFile> files = res.packages;
	files.push_back(res.index);
	files.push_back(res.descriptor_file);
	for (u32 i = 0; i < files.size(); ++i)
	{
		if (i) out += ",";
		out += "{\"name\":\"";
		JsonAppend(out, files[i].name.c_str());
		out += NqUtil::Format("\",\"size\":%llu,\"sha256\":\"%s\"}", files[i].size, files[i].sha256.c_str());
	}
	out += NqUtil::Format("],\"files\":%d", res.files);
	JsonField(out, "requires_game", res.requires_game.c_str());
	JsonField(out, "baked", started.bake_note.c_str());
	JsonField(out, "package_warnings", started.warnings.c_str());
	out += "}";
}
