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
xr_string	s_TagFor;					// the version the automatic tag was worked out for
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
		err = "no GitHub repository in the manifest: set it in Mod > Publish Release or Mod > Edit Manifest "
			  "(MCP: mod_set_manifest github=owner/repo)";
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

	// the used-tag list is a file: read it when the version changes, not every frame
	if (s_TagAuto && s_TagFor != s_Version)
	{
		s_TagFor = s_Version;
		strncpy_s(s_Tag, sizeof(s_Tag), FreshTag(s_Version).c_str(), _TRUNCATE);
	}
	if (s_TitleAuto)	strncpy_s(s_Title, sizeof(s_Title), DefaultTitle(s_ModuleId, s_ModuleName, s_Version).c_str(), _TRUNCATE);

	ImGui::SeparatorText("Release");
	ImGui::AlignTextToFramePadding(); ImGui::TextUnformatted("Tag"); ImGui::SameLine(label);
	ImGui::SetNextItemWidth(narrow);
	if (ImGui::InputText("##tag", s_Tag, sizeof(s_Tag))) s_TagAuto = false;
	Hint("Git tag GitHub creates for the release. The game does not read it.\n"
		 "A tag deleted within the last hour is not offered again: GitHub keeps\n"
		 "serving a deleted release's files under a reused tag for minutes.");

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

//------------------------------------------------------------------------------
// the releases that are already out: the list, and deleting one
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
	xr_string								note;		// what a deletion did
	bool									tag_kept;	// ...and whether it left the tag behind
	bool									listed;		// `releases` is GitHub's answer, not an empty start
	xr_vector<EditorGithub::SReleaseInfo>	releases;

	SManageView() : state(msIdle), op(moList), tag_kept(false), listed(false) {}
};

std::mutex				s_ManageLock;
SManageView				s_Manage;			// guarded by s_ManageLock
std::thread				s_ManageThread;
EditorGithub::CCancel	s_ManageCancel;

SManageView ManageSnapshot()
{
	std::lock_guard<std::mutex> guard(s_ManageLock);
	return s_Manage;
}

bool ManageRunning() { return msRunning == ManageSnapshot().state; }

void ManageReap()
{
	if (!ManageRunning() && s_ManageThread.joinable()) s_ManageThread.join();
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

	// whatever happened above, the window shows what GitHub has now
	xr_string list_error;
	const bool listed = EditorGithub::ListReleases(repo, token, s_ManageCancel, releases, list_error);
	if (error.empty()) error = list_error;
	EditorGithub::WipeString(token);

	std::lock_guard<std::mutex> guard(s_ManageLock);
	s_Manage.state		= error.empty() ? msDone : msFailed;
	s_Manage.error		= error;
	s_Manage.note		= note;
	s_Manage.tag_kept	= tag_kept;
	if (listed)
	{
		s_Manage.listed		= true;
		s_Manage.releases	= releases;
	}
}

bool ManageStart(const SManageRequest& rq, xr_string& err)
{
	err.clear();
	if (!EditorProject::Active())	{ err = "no active project"; return false; }
	Reap();
	ManageReap();
	if (JobRunning())				{ err = "a release is being published - wait for it to finish"; return false; }
	if (ManageRunning())			{ err = "the release list is busy - try again in a moment"; return false; }

	EditorMod::SManifest m;
	if (!EditorMod::Load(EditorProject::Root(), m)) { err = "mod.ltx not found in the project"; return false; }
	if (m.update_github.empty() || !EditorMod::ValidateGithubRepo(m.update_github.c_str()))
	{
		err = "no GitHub repository in the manifest: set it in Mod > Publish Release or Mod > Edit Manifest "
			  "(MCP: mod_set_manifest github=owner/repo)";
		return false;
	}
	EditorGithub::SAccount account;
	EditorGithub::ResolveAccount(account);
	if (EditorGithub::asNone == account.source)
	{
		err = "not signed in to GitHub: sign in from Mod > Publish Release, sign in to the GitHub CLI (gh auth login) "
			  "or set GH_TOKEN";
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
		s_Manage.note.clear();
		s_Manage.tag_kept	= false;
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

xr_string SizeText(u64 bytes)
{
	return bytes >= (1u << 20)	? NqUtil::Format("%.1f MB", double(bytes) / 1048576.0)
								: NqUtil::Format("%.0f KB", double(bytes) / 1024.0);
}

LPCSTR ReleaseStateText(const EditorGithub::SReleaseInfo& r)
{
	return r.draft ? "Draft" : r.prerelease ? "Pre-release" : r.latest ? "Latest" : "";
}

const char* kTitleReleases	= "XFined Editor - Releases";
const char* kTitleDelete	= "XFined Editor - Delete Release";
const char* kTitleEdit		= "XFined Editor - Edit Release";

bool							s_WantReleases	= false;
bool							s_WantDelete	= false;
bool							s_WantEdit		= false;
EditorGithub::SReleaseInfo		s_EditTarget;		// the release the edit window is about
char							s_EditTitle[256] = {};
xr_string						s_EditNotes;		// grows with the text: a release body runs to 125 000 characters
u64								s_SelectedId	= 0;
EditorGithub::SReleaseInfo		s_DeleteTarget;		// what the confirmation names
xr_string						s_DeleteNext;		// the release players fall back to, when the target is the latest

// the confirmation names this release; when it is the latest one, also what players get next
void PrepareDelete(const SManageView& view, const EditorGithub::SReleaseInfo& target)
{
	s_DeleteTarget = target;
	s_DeleteNext.clear();
	// the list is newest first: the first other release players may receive is the fallback
	for (u32 i = 0; i < view.releases.size() && s_DeleteNext.empty(); ++i)
	{
		const EditorGithub::SReleaseInfo& r = view.releases[i];
		if (r.id != target.id && !r.draft && !r.prerelease) s_DeleteNext = r.tag;
	}
	s_WantReleases = false;
	s_WantDelete = true;
}

void DrawReleases()
{
	if (!s_WantReleases) return;
	ManageReap();
	const SManageView view = ManageSnapshot();
	const float em = ImGui::GetFontSize();
	if (!ImGui::BeginPopupModal(kTitleReleases, &s_WantReleases, ImGuiWindowFlags_AlwaysAutoResize, true))
		return;

	const bool busy = msRunning == view.state;
	ImGui::Text("Repository: %s", view.repo.c_str());
	if (busy)
	{
		ImGui::SameLine();
		ImGui::TextDisabled(moDelete == view.op ? "- deleting..." : moEdit == view.op ? "- saving..." : "- loading...");
	}
	if (!view.note.empty())
		ImGui::TextColored(view.tag_kept ? ImVec4(1.f, 0.8f, 0.3f, 1.f) : ImVec4(0.6f, 0.9f, 0.6f, 1.f), "%s", view.note.c_str());
	if (!view.error.empty())
	{
		ImGui::PushTextWrapPos(em * 46.f);
		ImGui::TextColored(ImVec4(1.f, 0.4f, 0.4f, 1.f), "%s", view.error.c_str());
		ImGui::PopTextWrapPos();
	}

	const EditorGithub::SReleaseInfo* selected = 0;
	const ImGuiTableFlags flags = ImGuiTableFlags_RowBg | ImGuiTableFlags_BordersOuter | ImGuiTableFlags_BordersInnerV |
								  ImGuiTableFlags_ScrollY | ImGuiTableFlags_SizingStretchProp;
	if (ImGui::BeginTable("##releases", 5, flags, ImVec2(em * 46.f, em * 14.f)))
	{
		ImGui::TableSetupScrollFreeze(0, 1);
		ImGui::TableSetupColumn("Tag", 0, 2.f);
		ImGui::TableSetupColumn("Title", 0, 4.f);
		ImGui::TableSetupColumn("State", 0, 1.6f);
		ImGui::TableSetupColumn("Published (UTC)", 0, 2.6f);
		ImGui::TableSetupColumn("Files", 0, 2.2f);
		ImGui::TableHeadersRow();
		for (u32 i = 0; i < view.releases.size(); ++i)
		{
			const EditorGithub::SReleaseInfo& r = view.releases[i];
			ImGui::TableNextRow();
			ImGui::TableNextColumn();
			ImGui::PushID((int)i);
			const bool is_selected = r.id == s_SelectedId;
			if (ImGui::Selectable(r.tag.empty() ? "(no tag)" : r.tag.c_str(), is_selected, ImGuiSelectableFlags_SpanAllColumns))
				s_SelectedId = r.id;
			ImGui::PopID();
			if (is_selected) selected = &r;
			ImGui::TableNextColumn(); ImGui::TextUnformatted(r.title.c_str());
			ImGui::TableNextColumn(); ImGui::TextUnformatted(ReleaseStateText(r));
			ImGui::TableNextColumn(); ImGui::TextUnformatted(r.published.empty() ? "-" : r.published.c_str());
			ImGui::TableNextColumn(); ImGui::Text("%u, %s", (u32)r.assets.size(), SizeText(r.bytes).c_str());
		}
		ImGui::EndTable();
	}
	if (view.listed && view.releases.empty() && !busy)
		ImGui::TextDisabled("This repository has no releases yet.");

	ImGui::BeginDisabled(busy || !selected);
	if (ImGui::Button("Edit..."))
	{
		s_EditTarget = *selected;
		strncpy_s(s_EditTitle, sizeof(s_EditTitle), selected->title.c_str(), _TRUNCATE);
		s_EditNotes = selected->notes;
		s_WantReleases = false;
		s_WantEdit = true;
		ImGui::CloseCurrentPopup();
	}
	ImGui::EndDisabled();
	Hint("Title and notes of the selected release. Its files, its tag and the latest mark stay as they are.");
	ImGui::SameLine();
	ImGui::BeginDisabled(busy || !selected);
	if (ImGui::Button("Delete..."))
	{
		PrepareDelete(view, *selected);
		ImGui::CloseCurrentPopup();
	}
	ImGui::EndDisabled();
	Hint("Deletes the selected release and its git tag from GitHub. Asks first.");
	ImGui::SameLine();
	ImGui::BeginDisabled(busy);
	if (ImGui::Button("Refresh"))
	{
		xr_string err;
		if (!ManageStart(SManageRequest(), err))
		{
			std::lock_guard<std::mutex> guard(s_ManageLock);
			s_Manage.error = err;
		}
	}
	ImGui::EndDisabled();
	ImGui::SameLine();
	if (ImGui::Button(selected ? "Open Release Page" : "Open on GitHub"))
	{
		const xr_string url = selected ? selected->url : EditorGithub::ReleasesPage(view.repo.c_str());
		// GitHub's own pages only: the address of a release comes over the network
		if (0 == strncmp(url.c_str(), "https://github.com/", 19))
			::ShellExecuteA(0, "open", url.c_str(), 0, 0, SW_SHOWNORMAL);
	}
	ImGui::SameLine();
	if (ImGui::Button("Close"))
	{
		s_WantReleases = false;
		ImGui::CloseCurrentPopup();
	}
	ImGui::EndPopup();
}

void DrawDeleteConfirm()
{
	if (!s_WantDelete) return;
	const bool was_wanted = s_WantDelete;
	if (!ImGui::BeginPopupModal(kTitleDelete, &s_WantDelete, ImGuiWindowFlags_AlwaysAutoResize, true))
	{
		// the caption cross: back to the list, nothing deleted
		if (was_wanted && !s_WantDelete) s_WantReleases = true;
		return;
	}

	const EditorGithub::SReleaseInfo& r = s_DeleteTarget;
	const SManageView view = ManageSnapshot();
	ImGui::PushTextWrapPos(ImGui::GetFontSize() * 34.f);
	ImGui::Text("Delete %s %s \"%s\" from %s?", r.draft ? "draft release" : "release", r.tag.c_str(), r.title.c_str(),
		view.repo.c_str());
	ImGui::Spacing();
	ImGui::TextUnformatted("Removed from GitHub for good:");
	ImGui::BulletText("the release with its %u file(s), %s", (u32)r.assets.size(), SizeText(r.bytes).c_str());
	if (!r.draft) ImGui::BulletText("the git tag %s", r.tag.c_str());
	ImGui::Spacing();
	if (r.latest)
	{
		if (s_DeleteNext.empty())
			ImGui::TextColored(ImVec4(1.f, 0.8f, 0.3f, 1.f), "It is the latest release. Players are offered no update until another release is published.");
		else
			ImGui::TextColored(ImVec4(1.f, 0.8f, 0.3f, 1.f), "It is the latest release. GitHub then serves %s to players.", s_DeleteNext.c_str());
	}
	if (!r.draft && !r.prerelease)
		ImGui::TextUnformatted("Players who already have this version keep it. Publish the version again and the game offers it to them as a fix.");
	ImGui::TextUnformatted("GitHub keeps no copy: this cannot be undone.");
	ImGui::PopTextWrapPos();
	ImGui::Separator();

	// Cancel is the default: Enter and Escape both back out
	const bool confirm = ImGui::Button("Delete", ImVec2(120, 0));
	ImGui::SameLine();
	const bool cancel = ImGui::Button("Cancel", ImVec2(120, 0)) ||
						ImGui::IsKeyPressed(ImGuiKey_Escape, false) ||
						ImGui::IsKeyPressed(ImGuiKey_Enter, false);
	ImGui::SetItemDefaultFocus();

	if (confirm)
	{
		xr_string err;
		SManageRequest rq;
		rq.op = moDelete;
		rq.id = r.id;
		if (!ManageStart(rq, err))
		{
			std::lock_guard<std::mutex> guard(s_ManageLock);
			s_Manage.error = err;
		}
		s_SelectedId = 0;
	}
	if (confirm || cancel)
	{
		s_WantDelete = false;
		s_WantReleases = true;
		ImGui::CloseCurrentPopup();
	}
	ImGui::EndPopup();
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
	if (!s_WantEdit) return;
	const bool was_wanted = s_WantEdit;
	if (!ImGui::BeginPopupModal(kTitleEdit, &s_WantEdit, ImGuiWindowFlags_AlwaysAutoResize, true))
	{
		// the caption cross: back to the list, nothing saved
		if (was_wanted && !s_WantEdit) s_WantReleases = true;
		return;
	}

	const float em = ImGui::GetFontSize();
	const EditorGithub::SReleaseInfo& r = s_EditTarget;
	ImGui::Text("%s %s", r.draft ? "Draft release" : "Release", r.tag.c_str());
	ImGui::TextUnformatted("Title");
	ImGui::SetNextItemWidth(em * 34.f);
	ImGui::InputText("##title", s_EditTitle, sizeof(s_EditTitle));
	ImGui::TextUnformatted("Notes");
	ImGui::InputTextMultiline("##notes", &s_EditNotes[0], s_EditNotes.capacity() + 1, ImVec2(em * 34.f, em * 14.f),
		ImGuiInputTextFlags_CallbackResize, &GrowNotes, &s_EditNotes);
	Hint("What changed - shown on the release page. Markdown.");
	ImGui::TextDisabled("The files, the tag and the latest mark of the release stay as they are.");
	const bool too_long = s_EditNotes.size() > kMaxNotes;
	if (too_long)
		ImGui::TextColored(ImVec4(1.f, 0.8f, 0.3f, 1.f), "GitHub takes notes of 120 000 characters at most.");
	ImGui::Separator();

	ImGui::BeginDisabled(too_long);
	const bool save = ImGui::Button("Save", ImVec2(120, 0));
	ImGui::EndDisabled();
	ImGui::SameLine();
	const bool cancel = ImGui::Button("Cancel", ImVec2(120, 0));
	if (save)
	{
		SManageRequest rq;
		rq.op = moEdit;
		rq.id = r.id;
		rq.has_title = rq.has_notes = true;
		rq.title = s_EditTitle;
		rq.notes = s_EditNotes.c_str();	// up to the terminator: the buffer may run past the text
		xr_string err;
		if (!ManageStart(rq, err))
		{
			std::lock_guard<std::mutex> guard(s_ManageLock);
			s_Manage.error = err;
		}
	}
	if (save || cancel)
	{
		s_WantEdit = false;
		s_WantReleases = true;
		ImGui::CloseCurrentPopup();
	}
	ImGui::EndPopup();
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
	if (jsUploading == job.state)
		sprintf_s(dst, size, "Publishing %u%%", job.total ? (u32)(job.sent * 100 / job.total) : 0u);
	else
		strcpy_s(dst, size, "Publish Release");
}

void EditorPublish::RequestPublish()
{
	// one modal at a time: two self-opening popups close each other every frame
	if (!CanPublish() || s_WantReleases || s_WantDelete || s_WantEdit) return;
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
	s_TagFor.clear();
	s_TitleAuto		= true;
	s_Prerelease	= false;
	s_Latest		= true;
	s_ReplacedFor.clear();
	s_AccountError.clear();
	RefreshAccount();
	s_WantDialog	= true;
}

void EditorPublish::RequestReleases()
{
	if (!CanPublish() || s_WantDialog || s_WantResult || s_WantDelete || s_WantEdit) return;
	Reap();
	if (JobRunning()) { s_WantProgress = true; return; }
	if (s_WantProgress) return;

	xr_string err;
	if (!ManageStart(SManageRequest(), err))
	{
		ShowResult("Cannot list the releases: " + err);
		return;
	}
	s_SelectedId	= 0;
	s_WantReleases	= true;
}

void EditorPublish::DrawUI()
{
	Reap();
	if (!EditorProject::Active()) return;
	DrawDialog();
	DrawProgress();
	DrawReleases();
	DrawDeleteConfirm();
	DrawEditRelease();
	// a job that ends while the release list is up waits its turn
	if (!s_WantReleases && !s_WantDelete && !s_WantEdit) DrawResult();
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
	SManageRequest rq;
	rq.op = 0 == xr_strcmp(action, "delete") ? moDelete : 0 == xr_strcmp(action, "edit") ? moEdit : moList;
	if (moList == rq.op && 0 != xr_strcmp(action, "list"))
		{ out = "{\"ok\":false,\"error\":\"unknown action: list, edit, delete, status or cancel expected\"}"; return; }

	// MCP is the programmatic path: the caller has decided, nothing is asked
	if (moList != rq.op)
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
