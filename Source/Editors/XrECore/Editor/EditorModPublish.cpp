#include "stdafx.h"
#include "EditorModPublish.h"
#include "EditorGithub.h"
#include "EditorModManifest.h"
#include "EditorProject.h"
#include "XFinedMCP.h"
#include "ui_main.h"
#include "Nq/NqUtil.h"
#include <shellapi.h>
#include <thread>

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
enum EJobState { jsIdle, jsUploading, jsDone, jsFailed, jsCancelled };

struct SJobView
{
	EJobState	state;
	xr_string	id, version, repo, tag;
	xr_string	stage;
	u64			sent, total;		// bytes of the whole release
	xr_string	url, error, warnings;
	bool		served;

	SJobView() : state(jsIdle), sent(0), total(0), served(false) {}
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

void JobWorker(EditorGithub::SPublishRequest request)
{
	CSink sink(request);
	EditorGithub::SPublishResult result;
	xr_string err;
	const bool ok = EditorGithub::Publish(request, s_JobCancel, sink, result, err);
	EditorGithub::WipeString(request.token);

	std::lock_guard<std::mutex> guard(s_JobLock);
	s_Job.state		= ok ? jsDone : (err == "cancelled" ? jsCancelled : jsFailed);
	s_Job.url		= result.url;
	s_Job.warnings	= result.warnings;
	s_Job.served	= result.served;
	s_Job.error		= ok ? xr_string() : err;
	if (ok) s_Job.sent = s_Job.total;
}

SJobView JobSnapshot()
{
	std::lock_guard<std::mutex> guard(s_JobLock);
	return s_Job;
}

bool JobRunning() { return jsUploading == JobSnapshot().state; }

LPCSTR StateText(EJobState state)
{
	switch (state)
	{
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
const char* kTitlePublish	= "XFined Editor - Publish Release";
const char* kTitleProgress	= "XFined Editor - Publishing Release";
const char* kTitleResult	= "XFined Editor - Publish Result";

bool		s_WantDialog	= false;
bool		s_WantProgress	= false;
bool		s_WantResult	= false;
bool		s_PublishQueued	= false;
xr_string	s_ResultText;
xr_string	s_ResultUrl;

xr_string	s_ModuleId;
xr_string	s_ModuleName;
char		s_Repo[256]		= {};
char		s_Version[64]	= {};
char		s_Requires[32]	= {};
char		s_Tag[128]		= {};
char		s_Title[256]	= {};
char		s_Notes[16384]	= {};
char		s_TokenInput[300] = {};
bool		s_TagAuto		= true;		// follows the version until the author edits it
bool		s_TitleAuto		= true;
bool		s_Prerelease	= false;
bool		s_Latest		= true;
xr_vector<xr_string> s_Replaced;		// an earlier packaging of this version, by file
xr_string	s_ReplacedFor;				// the version that list was made for
EditorGithub::SAccount s_Account;
xr_string	s_AccountError;

// the dialog shows where the account comes from; the token itself is read again
// when a job starts and is not kept around in between
void RefreshAccount()
{
	EditorGithub::ResolveAccount(s_Account);
	EditorGithub::WipeString(s_Account.token);
}

void ShowResult(const xr_string& text, LPCSTR url = "")
{
	s_ResultText = text;
	s_ResultUrl = url;
	s_WantResult = true;
}

// The end of a job, seen from the main thread: the log line, and the result
// window when a person started it. An MCP caller reads the status instead.
void Reap()
{
	const SJobView job = JobSnapshot();
	if (jsUploading == job.state || jsIdle == job.state) return;
	if (s_JobThread.joinable()) s_JobThread.join();
	if (s_JobReported) return;
	s_JobReported = true;

	xr_string text;
	if (jsDone == job.state)
	{
		Msg("* [XMS] release %s %s published: %s", job.id.c_str(), job.version.c_str(), job.url.c_str());
		text = NqUtil::Format("Release %s %s is published in %s.\n%s", job.id.c_str(), job.version.c_str(),
			job.repo.c_str(), job.url.c_str());
		if (job.served) text += "\n\nThe game already offers it: players see the update in the Mods menu.";
	}
	else if (jsCancelled == job.state)
	{
		Msg("* [XMS] publishing %s %s cancelled", job.id.c_str(), job.version.c_str());
		text = "Publishing cancelled. Nothing was published; the draft stays on GitHub and the next attempt reuses it.";
	}
	else
	{
		Msg("! [XMS] publishing %s %s failed: %s", job.id.c_str(), job.version.c_str(), job.error.c_str());
		text = "Publishing failed: " + job.error;
	}
	if (!job.warnings.empty())
	{
		Msg("! [XMS] %s", job.warnings.c_str());
		text += "\n\n" + job.warnings;
	}
	s_WantProgress = false;
	if (s_JobFromUI) ShowResult(text, jsDone == job.state ? job.url.c_str() : "");
}

//------------------------------------------------------------------------------
// starting a job - the one path the dialog and MCP share
//------------------------------------------------------------------------------
struct SOptions
{
	xr_string	tag, title, notes;
	xr_string	requires_game;
	bool		requires_given;		// false = the linked game's version
	bool		prerelease, latest;

	SOptions() : requires_given(false), prerelease(false), latest(true) {}
};

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

	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m)) { err = "mod.ltx not found in the project"; return false; }
	if (m.update_github.empty())
	{
		err = "no GitHub repository in the manifest: set it in Mod > Publish Release or Mod > Edit Manifest "
			  "(MCP: mod_set_manifest github=owner/repo)";
		return false;
	}
	if (!EditorMod::ValidateGithubRepo(m.update_github.c_str()))
		{ err = "invalid [update] github in mod.ltx: owner/repo expected"; return false; }

	const xr_string tag		= options.tag.empty() ? "v" + m.version : options.tag;
	const xr_string title	= options.title.empty() ? DefaultTitle(m.id, m.name, m.version.c_str()) : options.title;
	if (!ValidTag(tag.c_str()))
		{ err = "invalid tag: 1-100 characters of A-Z a-z 0-9 . _ -, not starting with '-' or '.'"; return false; }
	if (title.size() > 200)				{ err = "the title is longer than 200 characters"; return false; }
	if (options.notes.size() > kMaxNotes)	{ err = "the notes are longer than 120 000 characters"; return false; }

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
	s_JobThread = std::thread(&JobWorker, std::move(request));
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
// the dialog
//------------------------------------------------------------------------------
void Hint(LPCSTR text)
{
	if (ImGui::IsItemHovered(ImGuiHoveredFlags_AllowWhenDisabled))
		ImGui::SetTooltip("%s", text);
}

void RefreshReplaced()
{
	if (s_ReplacedFor == s_Version) return;
	s_ReplacedFor = s_Version;
	EditorMod::ListPackagedFiles(EditorProject::Root(), "", s_ModuleId.c_str(), s_Version, s_Replaced);
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
	ShowResult(err == "cancelled" ? xr_string("Packaging cancelled. Nothing was published.")
								  : "Cannot publish the release: " + err);
}

// the repository and the version are manifest fields; the dialog edits them in place
bool SaveManifestFields(xr_string& err)
{
	xr_string repo;
	EditorMod::NormalizeGithubRepo(s_Repo, repo);
	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m)) { err = "mod.ltx not found in the project"; return false; }
	if (m.update_github == repo && m.version == s_Version) return true;
	m.update_github = repo;
	m.version = s_Version;
	if (EditorMod::Save(EditorProject::Root(), m)) return true;
	err = "cannot write mod.ltx";
	return false;
}

void DrawAccount()
{
	ImGui::SeparatorText("GitHub account");
	const SSignView sign = SignSnapshot();
	if (ssDone == sign.state)
	{
		if (s_SignThread.joinable()) s_SignThread.join();
		SignSet(ssIdle);
		RefreshAccount();
	}
	if (ssStarting == sign.state || ssWaiting == sign.state)
	{
		if (ssStarting == sign.state)
			ImGui::TextUnformatted("Contacting GitHub...");
		else
		{
			// GitHub's own page, and only that: the address came over the network
			const bool trusted = 0 == strncmp(sign.uri.c_str(), "https://github.com/", 19);
			if (!s_SignShown)
			{
				s_SignShown = true;
				ImGui::SetClipboardText(sign.user_code.c_str());
				if (trusted) ::ShellExecuteA(0, "open", sign.uri.c_str(), 0, 0, SW_SHOWNORMAL);
			}
			ImGui::Text("Enter this code on GitHub:  %s", sign.user_code.c_str());
			ImGui::TextDisabled("The code is in the clipboard. Waiting for the confirmation...");
			ImGui::BeginDisabled(!trusted);
			if (ImGui::Button("Open GitHub"))
				::ShellExecuteA(0, "open", sign.uri.c_str(), 0, 0, SW_SHOWNORMAL);
			ImGui::EndDisabled();
			ImGui::SameLine();
		}
		if (ImGui::Button("Cancel Sign-in")) SignStop();
		return;
	}

	ImGui::Text("Account: %s", EditorGithub::SourceText(s_Account.source));
	if (!s_Account.detail.empty())
	{
		ImGui::SameLine();
		ImGui::TextDisabled("(%s)", s_Account.detail.c_str());
	}
	if (ssFailed == sign.state && !sign.error.empty())
		ImGui::TextColored(ImVec4(1.f, 0.4f, 0.4f, 1.f), "%s", sign.error.c_str());

	if (EditorGithub::DeviceSignInAvailable())
	{
		if (ImGui::Button("Sign in with GitHub")) SignStart();
		Hint("Opens GitHub in the browser and asks for a one-time code.\nThe resulting token is kept in the Windows Credential Manager.");
	}
	if (EditorGithub::asSaved == s_Account.source)
	{
		if (EditorGithub::DeviceSignInAvailable()) ImGui::SameLine();
		if (ImGui::Button("Forget Token"))
		{
			EditorGithub::ForgetToken();
			RefreshAccount();
		}
		Hint("Removes the token the editor saved.");
	}

	// with an account in effect a pasted token is the exception, so it folds away
	const bool signed_in = EditorGithub::asNone != s_Account.source;
	if (signed_in && !ImGui::TreeNode("Use a personal access token")) return;
	ImGui::SetNextItemWidth(ImGui::GetFontSize() * 22.f);
	ImGui::InputTextWithHint("##token", "personal access token", s_TokenInput, sizeof(s_TokenInput), ImGuiInputTextFlags_Password);
	Hint("GitHub > Settings > Developer settings > Fine-grained tokens:\nthis repository, Contents = Read and write.\n"
		 "Kept in the Windows Credential Manager - never in the project.");
	ImGui::SameLine();
	ImGui::BeginDisabled(!s_TokenInput[0]);
	if (ImGui::Button("Save Token"))
	{
		if (EditorGithub::SaveToken(s_TokenInput, s_AccountError))
			RefreshAccount();
		SecureZeroMemory(s_TokenInput, sizeof(s_TokenInput));
	}
	ImGui::EndDisabled();
	if (!s_AccountError.empty())
		ImGui::TextColored(ImVec4(1.f, 0.4f, 0.4f, 1.f), "%s", s_AccountError.c_str());
	if (signed_in) ImGui::TreePop();
}

// a sign-in nobody is watching and a token nobody is going to save
void DialogCleanup()
{
	SignStop();
	SecureZeroMemory(s_TokenInput, sizeof(s_TokenInput));
}

void CloseDialog()
{
	DialogCleanup();
	s_WantDialog = false;
	ImGui::CloseCurrentPopup();
}

// MAJOR.MINOR.PATCH, what PackageRelease accepts for requires_game
bool GameVersionOk(LPCSTR v)
{
	int dots = 0;
	for (const char* p = v; *p; ++p) dots += ('.' == *p);
	return 2 == dots && EditorMod::ValidateVersion(v);
}

void DrawDialog()
{
	// the caption cross clears the flag without passing through CloseDialog
	static bool was_open = false;
	if (!s_WantDialog)
	{
		if (was_open) DialogCleanup();
		was_open = false;
		return;
	}
	was_open = true;
	ImGui::SetNextWindowSize(ImVec2(620, 0), ImGuiCond_FirstUseEver);
	if (!ImGui::BeginPopupModal(kTitlePublish, &s_WantDialog, ImGuiWindowFlags_AlwaysAutoResize, true))
		return;

	// measured, not guessed: the editor's font scale follows the display
	const float em = ImGui::GetFontSize();
	const float label = ImGui::CalcTextSize("Requires game").x + em, field = em * 26.f, narrow = em * 9.f;
	ImGui::Text("Module: %s", s_ModuleId.c_str());

	ImGui::AlignTextToFramePadding(); ImGui::TextUnformatted("Repository"); ImGui::SameLine(label);
	ImGui::SetNextItemWidth(field);
	ImGui::InputTextWithHint("##repo", "owner/repo or the repository address", s_Repo, sizeof(s_Repo));
	Hint("The GitHub repository whose latest release updates this module.\nSet once - it is stored in mod.ltx as [update] github.");

	ImGui::AlignTextToFramePadding(); ImGui::TextUnformatted("Version"); ImGui::SameLine(label);
	ImGui::SetNextItemWidth(narrow);
	ImGui::InputText("##version", s_Version, sizeof(s_Version));
	Hint("Version of this release, stored in mod.ltx.\nA player is offered the update when it is higher than the installed one.");

	ImGui::AlignTextToFramePadding(); ImGui::TextUnformatted("Requires game"); ImGui::SameLine(label);
	ImGui::SetNextItemWidth(narrow);
	ImGui::InputText("##requires", s_Requires, sizeof(s_Requires));
	Hint("Lowest Dead Air: Refined version this release runs on. Empty = any.");

	if (s_TagAuto)		sprintf_s(s_Tag, "v%s", s_Version);
	if (s_TitleAuto)	strncpy_s(s_Title, sizeof(s_Title), DefaultTitle(s_ModuleId, s_ModuleName, s_Version).c_str(), _TRUNCATE);

	ImGui::SeparatorText("Release");
	ImGui::AlignTextToFramePadding(); ImGui::TextUnformatted("Tag"); ImGui::SameLine(label);
	ImGui::SetNextItemWidth(narrow);
	if (ImGui::InputText("##tag", s_Tag, sizeof(s_Tag))) s_TagAuto = false;
	Hint("Git tag GitHub creates for the release. The game does not read it.");

	ImGui::AlignTextToFramePadding(); ImGui::TextUnformatted("Title"); ImGui::SameLine(label);
	ImGui::SetNextItemWidth(field);
	if (ImGui::InputText("##title", s_Title, sizeof(s_Title))) s_TitleAuto = false;

	ImGui::AlignTextToFramePadding(); ImGui::TextUnformatted("Notes"); ImGui::SameLine(label);
	ImGui::InputTextMultiline("##notes", s_Notes, sizeof(s_Notes), ImVec2(field, em * 6.f));
	Hint("What changed - shown on the release page. Markdown.");

	ImGui::Dummy(ImVec2(label - ImGui::GetStyle().ItemSpacing.x, 0)); ImGui::SameLine(label);
	ImGui::Checkbox("Pre-release", &s_Prerelease);
	Hint("Players do not receive pre-releases: the game reads the latest release alone.");
	ImGui::SameLine();
	ImGui::BeginDisabled(s_Prerelease);
	bool latest = s_Latest && !s_Prerelease;
	if (ImGui::Checkbox("Set as latest", &latest)) s_Latest = latest;
	ImGui::EndDisabled();
	Hint("The latest release is the one players update to.");

	DrawAccount();
	ImGui::Separator();

	xr_string repo;
	EditorMod::NormalizeGithubRepo(s_Repo, repo);
	LPCSTR problem = 0;
	if (repo.empty())											problem = "Set the GitHub repository.";
	else if (!EditorMod::ValidateGithubRepo(repo.c_str()))		problem = "Invalid repository: owner/repo expected.";
	else if (!EditorMod::ValidateVersion(s_Version))			problem = "Invalid version: 1-4 numbers separated by dots, e.g. 1.2.0.";
	else if (s_Requires[0] && !GameVersionOk(s_Requires))		problem = "Invalid game version: MAJOR.MINOR.PATCH, e.g. 1.4.2.";
	else if (!ValidTag(s_Tag))									problem = "Invalid tag: letters, digits, '.', '_' and '-'.";
	else if (EditorGithub::asNone == s_Account.source)			problem = "Sign in to GitHub first.";
	if (problem)
		ImGui::TextColored(ImVec4(1.f, 0.8f, 0.3f, 1.f), "%s", problem);

	RefreshReplaced();
	if (!s_Replaced.empty())
	{
		ImGui::TextDisabled("%u file(s) of an earlier packaging in releases\\%s go to the Recycle Bin:",
			(u32)s_Replaced.size(), s_Version);
		for (u32 i = 0; i < s_Replaced.size(); ++i)
			ImGui::TextDisabled("  %s", s_Replaced[i].c_str());
	}

	ImGui::BeginDisabled(0 != problem || s_PublishQueued);
	if (ImGui::Button("Publish", ImVec2(120, 0)))
	{
		xr_string err;
		if (SaveManifestFields(err))	s_PublishQueued = UI->DeferUIWork(&RunDeferredPublish);
		else							ShowResult("Cannot publish the release: " + err);
		CloseDialog();
	}
	ImGui::EndDisabled();
	Hint("Packages the module, uploads it as a draft, checks every file and only then publishes it.");
	ImGui::SameLine();
	if (ImGui::Button("Cancel", ImVec2(120, 0))) CloseDialog();
	ImGui::EndPopup();
}

void DrawProgress()
{
	if (!s_WantProgress) return;
	const SJobView job = JobSnapshot();
	if (jsUploading != job.state) { s_WantProgress = false; return; }
	ImGui::SetNextWindowSize(ImVec2(520, 0), ImGuiCond_FirstUseEver);
	if (!ImGui::BeginPopupModal(kTitleProgress, &s_WantProgress, ImGuiWindowFlags_AlwaysAutoResize, true))
		return;

	ImGui::Text("%s %s  ->  %s", job.id.c_str(), job.version.c_str(), job.repo.c_str());
	ImGui::TextUnformatted(job.stage.c_str());
	const float fraction = job.total ? float(double(job.sent) / double(job.total)) : 0.f;
	const xr_string overlay = NqUtil::Format("%.1f / %.1f MB", double(job.sent) / 1048576.0, double(job.total) / 1048576.0);
	ImGui::ProgressBar(fraction, ImVec2(ImGui::GetFontSize() * 28.f, 0), overlay.c_str());
	ImGui::Spacing();
	if (ImGui::Button("Cancel", ImVec2(120, 0))) s_JobCancel.Cancel();
	Hint("Stops the upload. Nothing is published; the draft stays on GitHub for the next attempt.");
	ImGui::SameLine();
	if (ImGui::Button("Hide", ImVec2(120, 0)))
	{
		s_WantProgress = false;
		ImGui::CloseCurrentPopup();
	}
	Hint("The upload goes on; the toolbar button shows it and brings this window back.");
	ImGui::EndPopup();
}

void DrawResult()
{
	if (!s_WantResult) return;
	if (!ImGui::BeginPopupModal(kTitleResult, &s_WantResult, ImGuiWindowFlags_AlwaysAutoResize, true))
		return;
	ImGui::PushTextWrapPos(ImGui::GetFontSize() * 36.f);
	ImGui::TextUnformatted(s_ResultText.c_str());
	ImGui::PopTextWrapPos();
	ImGui::Separator();
	if (!s_ResultUrl.empty())
	{
		if (ImGui::Button("Open Release Page"))
			::ShellExecuteA(0, "open", s_ResultUrl.c_str(), 0, 0, SW_SHOWNORMAL);
		ImGui::SameLine();
	}
	if (ImGui::Button("OK", ImVec2(120, 0)))
	{
		s_WantResult = false;
		ImGui::CloseCurrentPopup();
	}
	ImGui::EndPopup();
}

void StatusJson(xr_string& out)
{
	Reap();
	const SJobView job = JobSnapshot();
	const bool failed = jsFailed == job.state || jsCancelled == job.state;
	out = failed ? "{\"ok\":false" : "{\"ok\":true";
	JsonField(out, "state", StateText(job.state));
	out += jsUploading == job.state ? ",\"pending\":true" : ",\"pending\":false";
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
	out += "}";
}

// the source alone: a token never leaves through MCP
void AccountJson(xr_string& out)
{
	EditorGithub::SAccount account;
	EditorGithub::ResolveAccount(account);
	out = EditorGithub::asNone == account.source ? "{\"ok\":true,\"signed_in\":false" : "{\"ok\":true,\"signed_in\":true";
	JsonField(out, "source", EditorGithub::SourceText(account.source));
	out += EditorGithub::DeviceSignInAvailable() ? ",\"device_sign_in\":true}" : ",\"device_sign_in\":false}";
}

// The code is for a person: the caller shows it, the person confirms it on
// GitHub, and the token goes from GitHub straight into the Credential Manager.
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
	if (jsUploading == job.state)
		sprintf_s(dst, size, "Publishing %u%%", job.total ? (u32)(job.sent * 100 / job.total) : 0u);
	else
		strcpy_s(dst, size, "Publish Release");
}

void EditorPublish::RequestPublish()
{
	if (!CanPublish()) return;
	Reap();
	if (JobRunning()) { s_WantProgress = true; return; }

	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m))
	{
		ShowResult("Cannot publish the release: mod.ltx not found in the project.");
		return;
	}
	s_ModuleId		= m.id;
	s_ModuleName	= m.name;
	strncpy_s(s_Repo, sizeof(s_Repo), m.update_github.c_str(), _TRUNCATE);
	strncpy_s(s_Version, sizeof(s_Version), m.version.c_str(), _TRUNCATE);
	xr_string game_version;
	EditorMod::LinkedGameVersion(game_version);
	strncpy_s(s_Requires, sizeof(s_Requires), game_version.c_str(), _TRUNCATE);
	s_Notes[0]		= 0;
	s_TagAuto		= true;
	s_TitleAuto		= true;
	s_Prerelease	= false;
	s_Latest		= true;
	s_ReplacedFor.clear();
	s_AccountError.clear();
	RefreshAccount();
	s_WantDialog	= true;
}

void EditorPublish::DrawUI()
{
	Reap();
	if (!EditorProject::Active()) return;
	DrawDialog();
	DrawProgress();
	DrawResult();
}

void EditorPublish::Shutdown()
{
	s_JobCancel.Cancel();
	if (s_JobThread.joinable()) s_JobThread.join();
	s_SignCancel.Cancel();
	if (s_SignThread.joinable()) s_SignThread.join();
	EditorGithub::WipeString(s_Account.token);
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
	if (0 == xr_strcmp(action, "sign_out"))
	{
		SignStop();
		EditorGithub::ForgetToken();
		AccountJson(out);
		return;
	}
	if (0 != xr_strcmp(action, "start"))
	{
		out = "{\"ok\":false,\"error\":\"unknown action: start, status, cancel, account, sign_in, sign_in_status or sign_out expected\"}";
		return;
	}

	// request-sized, so over-long notes reach the check instead of reading as absent
	xr_vector<char> arg(xr_strlen(raw) + 1);
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
