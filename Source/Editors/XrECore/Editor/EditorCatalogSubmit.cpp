#include "stdafx.h"
#include "EditorCatalogSubmit.h"
#include "UI_ProgressCenter.h"
#include "EditorCatalog.h"
#include "EditorGithub.h"
#include "EditorModManifest.h"
#include "EditorUiContract.h"
#include "EditorProject.h"
#include "EditorVirusTotal.h"
#include "GrizzlyMCP.h"
#include "ui_main.h"
#include "Nq/NqUtil.h"
#include "Zlib/unzip.h"
#include <shellapi.h>
#include <shobjidl.h>
#include <thread>
#include <mutex>
#include <ctime>

namespace
{
using EditorCatalog::SFinding;
using EditorCatalog::ELevel;
using EditorCatalog::lvOk;
using EditorCatalog::lvNote;
using EditorCatalog::lvReview;
using EditorCatalog::lvRequired;
using EditorCatalog::lvBlock;

//------------------------------------------------------------------------------
// what the jobs know
//------------------------------------------------------------------------------
struct SPackage
{
	xr_string	name;
	u64			size;
	xr_string	sha256;

				SPackage	() : size(0) {}
};

// the latest release, as the game reads it
struct SRelease
{
	xr_string			text;			// the descriptor, verbatim
	xr_string			id, version;
	xr_vector<SPackage>	packages;
	bool				card;
	xr_string			website, thumbnail, tags;
	xr_string			signer;			// the key id the signature claims
	bool				signed_ok;		// by the author key
	xr_string			sign_error;

						SRelease	() : card(false), signed_ok(false) {}
};

struct SCheck
{
	bool				done, blocking;
	xr_string			version, url, key_id;
	bool				signed_;
	xr_vector<SFinding>	findings;

						SCheck	() : done(false), blocking(false), signed_(false) {}
};

// where the published index has the module
struct SListing
{
	xr_string	state;		// free, listed, withdrawn; unknown when the index did not answer; empty = not read yet
	xr_string	github;		// the repository the index names for it
	bool		this_key;	// bound to this computer's author key

				SListing	() : this_key(false) {}
};

// what a check learned beyond its findings
struct SChecked
{
	SRelease					rel;
	EditorCatalog::SVtPolicy	policy;		// the index's; the catalog's defaults when it did not answer
	EditorVirusTotal::SScan		vt;
};

// where the module stands with the catalog: the account's issues and the published index
struct SStanding
{
	EditorGithub::SCatalogIssue	submission, withdrawal;
	bool						submission_read;	// GitHub answered: `submission` is its word, not an empty start
	xr_string					submission_error;	// why it did not
	SListing					listing;

								SStanding	() : submission_read(false) {}
};

// taken on the main thread when a job starts; the secrets go when the job ends
struct SInput
{
	xr_string					root;
	EditorMod::SManifest		manifest;
	EditorCatalog::SKeyState	key;
	bool						signed_in;
	xr_string					token;
	xr_string					vt_key;		// empty = none

								SInput	() : signed_in(false) {}
								~SInput	() { EditorGithub::WipeString(token); EditorGithub::WipeString(vt_key); }
};

enum EJob { jbNone, jbCheck, jbVt, jbSubmit, jbSubmission, jbWithdraw };
enum EState { stIdle, stRunning, stDone, stFailed, stCancelled };

struct SView
{
	EJob						job;
	EState						state;
	xr_string					stage, error;
	xr_string					root, module_id;	// what the results below are about
	xr_string					login;
	SCheck						check;
	EditorVirusTotal::SScan		virustotal;		// what VirusTotal was asked last, by any job
	bool						vt_scanned;		// by a scan, not by the check's lookups
	EditorGithub::SCatalogIssue	submission;
	bool						submission_read;	// looked up: "no submission" is known, not assumed
	xr_string					submission_error;	// why the last look-up failed
	EditorGithub::SCatalogIssue	withdrawal;
	SListing					listing;
	int							step	= 0;	// of a submission: check, scan, send

								SView	() : job(jbNone), state(stIdle), vt_scanned(false), submission_read(false) {}
	bool						Listed	() const { return listing.state == "listed" && listing.this_key; }
};

std::mutex				s_Lock;
SView					s_View;			// guarded by s_Lock
bool					s_Want			= false;	// the window is open
std::thread				s_Thread;
EditorGithub::CCancel	s_Cancel;
bool					s_FromUI		= false;	// the Task Center's row of a job an MCP caller started opens nothing
u32						s_JobStarted	= 0;		// GetTickCount

SView Snapshot()
{
	std::lock_guard<std::mutex> guard(s_Lock);
	return s_View;
}

bool Running() { return stRunning == Snapshot().state; }

void Stage(LPCSTR text)
{
	std::lock_guard<std::mutex> guard(s_Lock);
	s_View.stage = text;
}

void Step(int step)
{
	std::lock_guard<std::mutex> guard(s_Lock);
	s_View.step = step;
}

LPCSTR JobName(EJob job)
{
	switch (job)
	{
	case jbCheck:	return "check";
	case jbVt:		return "vt";
	case jbSubmit:	return "submit";
	case jbSubmission:	return "submission";
	case jbWithdraw:	return "withdraw";
	}
	return "";
}

LPCSTR StateName(EState state)
{
	switch (state)
	{
	case stRunning:		return "running";
	case stDone:		return "done";
	case stFailed:		return "failed";
	case stCancelled:	return "cancelled";
	}
	return "idle";
}

xr_string Owner(const xr_string& repo) { return repo.substr(0, repo.find('/')); }

// what GitHub or the network gave as the reason, for a sentence that ends after it: without its own period
xr_string Reason(xr_string text)
{
	while (!text.empty() && (text.back() == '.' || text.back() == ' ')) text.pop_back();
	return text;
}

void Add(SCheck& check, ELevel level, LPCSTR code, const xr_string& text)
{
	check.findings.push_back(SFinding(level, code, text));
	check.blocking |= lvBlock == level;
}

//------------------------------------------------------------------------------
// reading what is out there
//------------------------------------------------------------------------------
bool FetchRelease(const SInput& in, SRelease& rel, u32& status, xr_string& err)
{
	rel = SRelease();
	const xr_string url = EditorGithub::LatestDownload(in.manifest.update_github, in.manifest.id + ".update.ltx");
	if (!EditorGithub::FetchPublic(url, 256u << 10, s_Cancel, status, rel.text, err)) return false;
	if (status != 200) { err = NqUtil::Format("GitHub answered %u", status); return false; }

	EditorCatalog::IniValue(rel.text, "release", "id", rel.id);
	EditorCatalog::IniValue(rel.text, "release", "version", rel.version);
	xr_vector<std::pair<xr_string, xr_string> > rows;
	EditorCatalog::IniSection(rel.text, "packages", rows);
	for (u32 i = 0; i < rows.size(); ++i)
	{
		SPackage p;
		p.name = rows[i].first;
		const size_t comma = rows[i].second.find(',');
		p.size = _strtoui64(rows[i].second.c_str(), 0, 10);
		p.sha256 = comma == xr_string::npos ? xr_string() : rows[i].second.substr(comma + 1);
		while (!p.sha256.empty() && p.sha256[0] == ' ') p.sha256.erase(0, 1);
		for (size_t c = 0; c < p.sha256.size(); ++c) p.sha256[c] = char(tolower((unsigned char)p.sha256[c]));
		rel.packages.push_back(p);
	}
	EditorCatalog::IniSection(rel.text, "card", rows);
	rel.card = !rows.empty();
	for (u32 i = 0; i < rows.size(); ++i)
	{
		if (rows[i].first == "website")		rel.website = EditorCatalog::Unquote(rows[i].second);
		if (rows[i].first == "thumbnail")	rel.thumbnail = rows[i].second;
		if (rows[i].first == "tags")		rel.tags = rows[i].second;
	}
	// the signer is named whatever the outcome: "signed with another key" needs it
	rel.signed_ok = EditorCatalog::Verify(rel.text, in.key.public_key, rel.signer, rel.sign_error);
	return true;
}

// the release's packages the way VirusTotal gets them: this computer's copies from releases\<version>
xr_vector<EditorVirusTotal::SPackage> ScanPackages(const SInput& in, const SRelease& rel)
{
	xr_vector<EditorVirusTotal::SPackage> out;
	for (u32 p = 0; p < rel.packages.size(); ++p)
	{
		EditorVirusTotal::SPackage package;
		package.name	= rel.packages[p].name;
		package.path	= in.root + "\\releases\\" + rel.version + "\\" + rel.packages[p].name;
		package.size	= rel.packages[p].size;
		package.sha256	= rel.packages[p].sha256;
		out.push_back(package);
	}
	return out;
}

// A report is required of every package: clean passes, warn is reported to players. A block, a
// package too large for VirusTotal and one even a scan could not get there refuse; anything else -
// not scanned yet, still analysing, no answer - is the requirement still open, never a mere note.
ELevel VtLevel(const xr_string& state, bool scanned)
{
	if (state == "clean")	return lvOk;
	if (state == "warn")	return lvReview;
	if (EditorVirusTotal::Refuses(state) && (scanned || state != "unknown"))	return lvBlock;
	return lvRequired;
}

// VirusTotal has a report page of a package only once it finished analysing it
bool VtHasReport(const EditorVirusTotal::SResult& r)
{
	return r.state == "clean" || r.state == "warn" || r.state == "block";
}

// every package has a report the catalog lists by
bool VtSettled(const EditorVirusTotal::SScan& vt)
{
	return vt.state == "clean" || vt.state == "warn";
}

// the n-th comma-separated field of an index line, trimmed
xr_string Field(const xr_string& value, u32 n)
{
	size_t from = 0;
	for (u32 i = 0; i < n; ++i)
	{
		from = value.find(',', from);
		if (from == xr_string::npos) return xr_string();
		++from;
	}
	const size_t to = value.find(',', from);
	xr_string text = value.substr(from, to == xr_string::npos ? xr_string::npos : to - from);
	while (!text.empty() && text[0] == ' ') text.erase(0, 1);
	while (!text.empty() && text.back() == ' ') text.pop_back();
	return text;
}

// [mods] and [withdrawn] lines alike: github, key, version, date
SListing ReadListing(bool reached, const xr_string& index, const xr_string& id, const xr_string& public_key)
{
	SListing out;
	out.state = reached ? "free" : "unknown";
	for (LPCSTR section : { "mods", "withdrawn" })
	{
		xr_vector<std::pair<xr_string, xr_string> > rows;
		if (reached) EditorCatalog::IniSection(index, section, rows);
		for (u32 i = 0; i < rows.size(); ++i)
			if (rows[i].first == id)
			{
				out.state = 0 == xr_strcmp(section, "mods") ? "listed" : "withdrawn";
				out.github = Field(rows[i].second, 0);
				out.this_key = !public_key.empty() && Field(rows[i].second, 1) == public_key;
				return out;
			}
	}
	return out;
}

// the release's folder on this computer, when it holds exactly what was published
xr_string LocalReleaseDir(const SInput& in, const SRelease& rel)
{
	const xr_string dir = in.root + "\\releases\\" + rel.version;
	xr_string local;
	HANDLE f = ::CreateFileA((dir + "\\" + in.manifest.id + ".update.ltx").c_str(), GENERIC_READ, FILE_SHARE_READ, 0,
		OPEN_EXISTING, 0, 0);
	if (INVALID_HANDLE_VALUE == f) return xr_string();
	char buf[8192];
	DWORD got = 0;
	while (local.size() <= rel.text.size() && ::ReadFile(f, buf, sizeof(buf), &got, 0) && got) local.append(buf, got);
	::CloseHandle(f);
	return local == rel.text ? dir : xr_string();
}

// The content rules over the release's own files, read out of this computer's copy of the
// packages - the bytes the descriptor pins, checked by their hashes first.
void CheckLocalRelease(const SInput& in, const SRelease& rel, SCheck& check)
{
	const xr_string dir = LocalReleaseDir(in, rel);
	if (dir.empty())
	{
		Add(check, lvNote, "content", "Content not checked here: this computer has no copy of release " + rel.version +
			" (the catalog checks it when it judges the submission).");
		return;
	}
	const xr_string prefix = "modules/" + in.manifest.id + "/";
	xr_vector<SFinding> found;
	u32 files = 0;
	xr_vector<char> data;
	xr_vector<char> name(0x10000);
	for (u32 p = 0; p < rel.packages.size() && !s_Cancel.Cancelled(); ++p)
	{
		const xr_string zip_path = dir + "\\" + rel.packages[p].name;
		Stage(("Checking " + rel.packages[p].name).c_str());
		xr_string sha;
		u64 size = 0;
		if (!EditorVirusTotal::FileSha256(zip_path, s_Cancel, sha, size) || sha != rel.packages[p].sha256)
		{
			Add(check, lvNote, "content", "Content not checked here: the local " + rel.packages[p].name +
				" is not the published one (the catalog checks it when it judges the submission).");
			return;
		}
		unzFile zip = unzOpen64(zip_path.c_str());
		if (!zip) { Add(check, lvNote, "content", "Content not checked here: " + rel.packages[p].name + " does not open."); return; }
		for (int res = unzGoToFirstFile(zip); UNZ_OK == res && !s_Cancel.Cancelled(); res = unzGoToNextFile(zip))
		{
			unz_file_info64 info;
			if (UNZ_OK != unzGetCurrentFileInfo64(zip, &info, name.data(), (uLong)name.size() - 1, 0, 0, 0, 0)) break;
			name[info.size_filename < name.size() ? info.size_filename : name.size() - 1] = 0;
			const xr_string entry = name.data();
			if (entry.empty() || entry.back() == '/') continue;
			if (0 != entry.compare(0, prefix.size(), prefix))
			{
				found.push_back(SFinding(lvBlock, "content", entry + ": outside modules/" + in.manifest.id + "/"));
				continue;
			}
			const xr_string path = entry.substr(prefix.size());
			const size_t total = (size_t)info.uncompressed_size;
			size_t got = 0;
			if (UNZ_OK == unzOpenCurrentFile(zip))
			{
				for (size_t want = EditorCatalog::ContentHead(path.c_str(), total); want > got;
					 want = EditorCatalog::ContentNeed(data.data(), got, total))
				{
					data.resize(want);
					const int read = unzReadCurrentFile(zip, data.data() + got, (unsigned)(want - got));
					if (read <= 0) break;
					got += (size_t)read;
				}
				unzCloseCurrentFile(zip);
			}
			EditorCatalog::CheckContent(path.c_str(), got ? data.data() : "", got, found);
			++files;
		}
		unzClose(zip);
	}
	if (s_Cancel.Cancelled()) return;
	u32 refused = 0;
	for (u32 i = 0; i < found.size(); ++i)
		refused += lvBlock == found[i].level;
	if (!refused)
		Add(check, lvOk, "content", NqUtil::Format("Content: %u file(s) checked, nothing the catalog refuses.", files));
	for (u32 i = 0; i < found.size(); ++i)
		Add(check, found[i].level, found[i].code.c_str(), found[i].text);
}

//------------------------------------------------------------------------------
// the jobs
//------------------------------------------------------------------------------
const char* kNoVtKey = "No VirusTotal key: the catalog lists a package only when VirusTotal knows it. Create a free account "
	"at virustotal.com and paste its API key into the VirusTotal section below.";

// Section 7, in the order the window lists it. `lookup` asks VirusTotal about the packages; a
// submission leaves that to its scan, which follows at once.
void RunCheck(const SInput& in, bool lookup, SCheck& check, SChecked& seen, SStanding& standing)
{
	check = SCheck();
	const EditorMod::SManifest& m = in.manifest;
	SRelease& rel = seen.rel;

	if (!in.key.exists)				Add(check, lvBlock, "key", "No author key: create one above - it signs every release.");
	else if (!in.key.error.empty())	Add(check, lvBlock, "key", "The author key does not read: " + in.key.error + ".");
	else							Add(check, lvOk, "key", "Author key " + in.key.key_id + ".");

	const bool repo_ok = !m.update_github.empty() && EditorMod::ValidateGithubRepo(m.update_github.c_str());
	if (m.update_github.empty())	Add(check, lvBlock, "repository", "No GitHub repository in the manifest: set it in Mod > Mod Settings.");
	else if (!repo_ok)				Add(check, lvBlock, "repository", "The manifest's GitHub repository is not owner/repo: " + m.update_github + ".");
	else							Add(check, lvOk, "repository", "Repository " + m.update_github + ".");

	// the submission has to come from the account that owns the repository (7.6)
	xr_string login;
	if (!in.signed_in)
		Add(check, lvBlock, "account", "Not signed in to GitHub: sign in from Mod > Publish Release.");
	else
	{
		Stage("Reading the GitHub account");
		xr_string err;
		if (!EditorGithub::AccountLogin(in.token, s_Cancel, login, err))
			Add(check, lvBlock, "account", "The GitHub account could not be read: " + Reason(err) + ".");
		else if (repo_ok && 0 != _stricmp(login.c_str(), Owner(m.update_github).c_str()))
			Add(check, lvBlock, "account", "Signed in as " + login + ", but " + m.update_github + " belongs to " +
				Owner(m.update_github) + ": the submission has to come from the repository's owner.");
		else
			Add(check, lvOk, "account", "Signed in to GitHub as " + login + ".");
		std::lock_guard<std::mutex> guard(s_Lock);
		s_View.login = login;
	}
	if (s_Cancel.Cancelled()) return;

	// the release, fetched the way the game fetches it
	bool have = false;
	if (repo_ok)
	{
		Stage("Reading the latest release");
		u32 status = 0;
		xr_string err;
		have = FetchRelease(in, rel, status, err);
		if (!have && !s_Cancel.Cancelled())
			Add(check, lvBlock, "release", 404 == status ?
				"The latest release of " + m.update_github + " carries no " + m.id + ".update.ltx: publish one first (Mod > Publish Release)." :
				"The latest release could not be read: " + Reason(err) + ".");
	}
	if (s_Cancel.Cancelled()) return;
	if (have)
	{
		check.version	= rel.version;
		check.url		= EditorGithub::RepoPage(m.update_github) + "/releases/latest";
		check.key_id	= rel.signer;
		if (rel.id != m.id)	Add(check, lvBlock, "release", "The latest release of " + m.update_github + " is for " + rel.id + ", not " + m.id + ".");
		else				Add(check, lvOk, "release", "Latest release " + rel.version + ".");
		if (EditorCatalog::CompareVersions(rel.version.c_str(), "1.0.0") < 0)
			Add(check, lvBlock, "version", "Release " + rel.version + " is below 1.0.0: the Mod Browser lists 1.0.0 and later.");
		else
			Add(check, lvOk, "version", "Version " + rel.version + " is 1.0.0 or later.");
		if (!m.version.empty() && 0 != EditorCatalog::CompareVersions(m.version.c_str(), rel.version.c_str()))
			Add(check, lvNote, "manifest_version", "mod.ltx says " + m.version + "; the catalog reads the latest release, " + rel.version + ".");
		if (rel.card)	Add(check, lvOk, "card", "The release carries a Mod Browser card.");
		else			Add(check, lvBlock, "card", "The latest release has no Mod Browser card: package and publish it with this editor.");
		check.signed_ = rel.signed_ok;
		if (in.key.public_key.empty())			Add(check, lvNote, "signature", "The signature is checked once there is an author key.");
		else if (rel.signed_ok)					Add(check, lvOk, "signature", "Signed with this author key.");
		else if (rel.signer.empty())			Add(check, lvBlock, "signature", "The latest release is not signed: unlock the author key, then publish again.");
		else if (rel.signer != in.key.key_id)	Add(check, lvBlock, "signature", "The latest release is signed with another key (" + rel.signer + ").");
		else									Add(check, lvBlock, "signature", "The release's signature does not verify: " + rel.sign_error + ".");
	}

	// the website the Mod Browser shows: the card's; the manifest's while there is no card
	const xr_string website = (have && rel.card && !rel.website.empty()) ? rel.website : m.website;
	const bool moddb = xr_string::npos != website.find("moddb.com");
	if (website.empty())
		Add(check, lvBlock, "website", "No website: the Mod Browser lists a module with its own page on AP-PRO or ModDB.");
	else if (!EditorCatalog::WebsiteIsModulePage(website.c_str()))
		Add(check, lvBlock, "website", "Website " + website + " is not the module's own page on AP-PRO or ModDB, "
			"e.g. https://ap-pro.ru/stuff/<section>/<name>-r<number>/.");
	else if (EditorGithub::QaRun())
		Add(check, lvNote, "website", "Website " + website + " - not opened in a QA run.");
	else
	{
		Stage("Opening the website");
		u32 status = 0;
		xr_string body, err;
		EditorGithub::FetchPublic(website, 8u << 20, s_Cancel, status, body, err);
		if (200 == status)									Add(check, lvOk, "website", "Website " + website + " answers.");
		else if (moddb && (403 == status || 429 == status || 503 == status))
			Add(check, lvNote, "website", "ModDB turns automated requests away: the page cannot be checked automatically.");
		else if (status)									Add(check, lvBlock, "website", NqUtil::Format("Website %s answers HTTP %u.", website.c_str(), status));
		else												Add(check, lvNote, "website", "Website " + website + " could not be reached: " + Reason(err) + ".");
	}
	if (s_Cancel.Cancelled()) return;

	if (have) CheckLocalRelease(in, rel, check);
	if (s_Cancel.Cancelled()) return;

	// the catalog: is the id free, or already this key's (7.7) - a withdrawn one stays bound to its key
	Stage("Reading the Mod Browser catalog");
	xr_string index;
	const bool reached = EditorGithub::FetchCatalogIndex(s_Cancel, index);
	if (reached) EditorCatalog::ReadVtPolicy(index, seen.policy);
	standing.listing = ReadListing(reached, index, m.id, in.key.public_key);
	const SListing& listing = standing.listing;
	if (!reached)
		Add(check, lvNote, "catalog", "The Mod Browser catalog did not answer: the catalog's own check covers the id.");
	else if (listing.state == "free")
		Add(check, lvOk, "catalog", "The id " + m.id + " is free in the Mod Browser.");
	else if (listing.state == "listed" && listing.this_key)
		Add(check, lvOk, "catalog", "Listed in the Mod Browser with this key.");
	else if (listing.state == "listed")
		Add(check, lvBlock, "catalog", m.id + " is listed with another author key: sign the release with that key, or submit "
			"the module under another id.");
	else if (listing.this_key)
		Add(check, lvNote, "catalog", "Withdrawn from the Mod Browser: submitting lists it again.");
	else
		Add(check, lvBlock, "catalog", m.id + " was withdrawn from the Mod Browser and stays bound to another author key: sign "
			"the release with that key, or submit the module under another id.");
	xr_vector<std::pair<xr_string, xr_string> > revoked;
	EditorCatalog::IniSection(index, "revoked", revoked);
	for (u32 i = 0; i < revoked.size(); ++i)
		if (revoked[i].first == m.id)
		{
			const size_t comma = revoked[i].second.find(',');
			xr_string version = revoked[i].second.substr(0, comma), reason = comma == xr_string::npos ? xr_string() : revoked[i].second.substr(comma + 1);
			while (!version.empty() && version.back() == ' ') version.pop_back();
			while (!reason.empty() && reason[0] == ' ') reason.erase(0, 1);
			Add(check, version == "*" ? lvBlock : lvNote, "catalog", (version == "*" ? "The Mod Browser revoked " + m.id :
				"The Mod Browser revoked version " + version) + ": " + reason);
		}

	if (have && rel.card)
	{
		if (rel.thumbnail.empty())	Add(check, lvNote, "thumbnail", "No thumbnail: the Mod Browser shows the name's first letter (Mod > Mod Settings).");
		else						Add(check, lvOk, "thumbnail", "Thumbnail.");
		if (rel.tags.empty())		Add(check, lvNote, "tags", "No tags: players who filter by tag will not find it (Mod > Mod Settings).");
		else						Add(check, lvOk, "tags", "Tags: " + rel.tags + ".");
	}

	// VirusTotal (7.10), required: the catalog only looks packages up and refuses one VirusTotal does
	// not know, so the scan is the author's, with their own key
	if (have && (lookup || in.vt_key.empty()))
		EditorVirusTotal::Lookup(ScanPackages(in, rel), in.vt_key, seen.policy, s_Cancel, &Stage, seen.vt);
	if (have && in.vt_key.empty())
		Add(check, lvBlock, "virustotal", kNoVtKey);
	else if (have && lookup)
		for (u32 p = 0; p < seen.vt.packages.size(); ++p)
		{
			const EditorVirusTotal::SResult& r = seen.vt.packages[p];
			xr_string text = EditorVirusTotal::Describe(r);
			if (r.state == "unknown")			text += " - a report is required: Submit scans it first.";
			else if (r.state == "analysing")	text += " - a report is required: Submit waits for it.";
			else if (r.state == "unavailable")	text += " - a report is required: Submit asks again.";
			else								text += ".";
			Add(check, VtLevel(r.state, false), "virustotal", text);
		}
	if (s_Cancel.Cancelled()) return;

	// and where the submission and a withdrawal stand
	if (in.signed_in)
	{
		Stage("Reading the submission");
		standing.submission_read = EditorGithub::CatalogIssue(m.id, in.token, s_Cancel, standing.submission, standing.submission_error);
		xr_string err;
		if (!s_Cancel.Cancelled()) EditorGithub::CatalogIssue(m.id, in.token, s_Cancel, standing.withdrawal, err, EditorGithub::ciWithdrawal);
	}
	check.done = !s_Cancel.Cancelled();
}

void RunVt(const SInput& in, EditorVirusTotal::SScan& vt, xr_string& err)
{
	Stage("Reading the latest release");
	SRelease rel;
	u32 status = 0;
	if (!FetchRelease(in, rel, status, err))
	{
		if (404 == status) err = "no release to scan: the latest release of " + in.manifest.update_github + " carries no " +
			in.manifest.id + ".update.ltx";
		return;
	}
	Stage("Reading the Mod Browser catalog");
	EditorCatalog::SVtPolicy policy;
	xr_string index;
	if (EditorGithub::FetchCatalogIndex(s_Cancel, index)) EditorCatalog::ReadVtPolicy(index, policy);
	EditorVirusTotal::Scan(ScanPackages(in, rel), in.vt_key, policy, s_Cancel, &Stage, vt);
	if (s_Cancel.Cancelled()) err = "cancelled";
}

xr_string SubmissionText(const SInput& in, const SRelease& rel, const xr_string& website)
{
	return "[mod]\nid      = " + in.manifest.id + "\ngithub  = " + in.manifest.update_github + "\nkey     = " +
		in.key.public_key + "\nversion = " + rel.version + "\nwebsite = " + website + "\n";
}

// what the issue says after the file: the module, the links, the verdicts, the editor's check
xr_string SubmissionDetails(const SInput& in, const SChecked& seen, const SCheck& check, const xr_string& website)
{
	const EditorMod::SManifest& m = in.manifest;
	xr_string body = "**" + (m.name.empty() ? m.id : m.name) + "**" + (m.author.empty() ? xr_string() : " by " + m.author) +
		" - `" + m.id + "` " + seen.rel.version + "\n\n";
	body += "- Repository: " + EditorGithub::RepoPage(m.update_github) + "\n";
	body += "- Website: " + website + "\n";
	body += "- Author key: `" + in.key.key_id + "`\n\n";
	body += "VirusTotal:\n";
	for (u32 p = 0; p < seen.vt.packages.size(); ++p)
	{
		const EditorVirusTotal::SResult& r = seen.vt.packages[p];
		const xr_string verdict = r.state == "clean" || r.state == "warn" || r.state == "block" ?
			NqUtil::Format("%s, %u engines", r.state.c_str(), r.verdict.engines) : EditorVirusTotal::Describe(r);
		body += "- `" + r.package.name + "`: " + verdict + " - " + EditorGithub::VtReportPage(r.package.sha256) + "\n";
	}
	body += "\nChecked by Grizzly Editor:\n";
	for (u32 i = 0; i < check.findings.size(); ++i)
		body += xr_string("- ") + EditorCatalog::LevelName(check.findings[i].level) + ": " + check.findings[i].text + "\n";
	return body;
}

// why the scan keeps the submission back, and what helps
xr_string VtRefusal(const EditorVirusTotal::SResult& r, const SChecked& seen)
{
	const xr_string what = EditorVirusTotal::Describe(r);
	if (r.state == "block")		return what + ": the catalog would refuse it";
	if (r.state == "too_large")	return what + ": package the release again - Package Release keeps every package under "
									"that size unless a single file alone is larger";
	if (r.state == "unknown")	return what + ": put the published files into releases\\" + seen.rel.version +
									", or upload the package at virustotal.com";
	if (r.state == "analysing")	return what + ": submit again in a few minutes";
	if (r.state == "unavailable")	return what + ": submit again when VirusTotal answers";
	return what;
}

bool RunSubmit(const SInput& in, SCheck& check, SChecked& seen, SStanding& standing, xr_string& err)
{
	Step(0);
	RunCheck(in, false, check, seen, standing);
	if (s_Cancel.Cancelled()) { err = "cancelled"; return false; }
	for (u32 i = 0; i < check.findings.size(); ++i)
		if (lvBlock == check.findings[i].level) { err = "not submitted: " + check.findings[i].text; return false; }
	Step(1);

	// The catalog only looks the packages up: what VirusTotal does not know goes up now, with the
	// author's key, and its report is waited for. Only a package with a report is sent - VirusTotal
	// answers 404 for a file until its first analysis ends, and the catalog refuses a 404 as a
	// package nobody scanned.
	EditorVirusTotal::Scan(ScanPackages(in, seen.rel), in.vt_key, seen.policy, s_Cancel, &Stage, seen.vt);
	if (s_Cancel.Cancelled()) { err = "cancelled"; return false; }
	for (u32 p = 0; p < seen.vt.packages.size(); ++p)
		Add(check, VtLevel(seen.vt.packages[p].state, true), "virustotal", EditorVirusTotal::Describe(seen.vt.packages[p]) + ".");
	const EditorVirusTotal::SResult* deciding = EditorVirusTotal::Deciding(seen.vt);
	if (deciding && seen.vt.state != "clean" && seen.vt.state != "warn")
	{
		err = "not submitted: " + VtRefusal(*deciding, seen) + ".";
		return false;
	}

	Step(2);
	const SRelease& rel = seen.rel;
	const xr_string website = (rel.card && !rel.website.empty()) ? rel.website : in.manifest.website;
	EditorGithub::SCatalogRequest rq;
	rq.module_id	= in.manifest.id;
	rq.file_text	= SubmissionText(in, rel, website);
	rq.details		= SubmissionDetails(in, seen, check, website);
	rq.token		= in.token;
	EditorGithub::SCatalogIssue sent;
	const bool ok = EditorGithub::CatalogSubmit(rq, s_Cancel, [](LPCSTR text) { Stage(text); }, sent, err);
	EditorGithub::WipeString(rq.token);
	// a failure may come after the issue went out: where the submission stands is not known then
	if (ok) standing.submission = sent;
	standing.submission_read = ok;
	standing.submission_error.clear();
	return ok;
}

// the published index's word on the module
void ReadListingNow(const SInput& in, SStanding& standing)
{
	Stage("Reading the Mod Browser catalog");
	xr_string index;
	const bool reached = EditorGithub::FetchCatalogIndex(s_Cancel, index);
	standing.listing = ReadListing(reached, index, in.manifest.id, in.key.public_key);
}

// where the submission, a withdrawal and the listing stand
bool RunStanding(const SInput& in, SStanding& standing, xr_string& err)
{
	Stage("Reading the submission");
	if (!EditorGithub::CatalogIssue(in.manifest.id, in.token, s_Cancel, standing.submission, err)) return false;
	standing.submission_read = true;
	Stage("Reading the withdrawal");
	if (!EditorGithub::CatalogIssue(in.manifest.id, in.token, s_Cancel, standing.withdrawal, err, EditorGithub::ciWithdrawal))
		return false;
	ReadListingNow(in, standing);
	return true;
}

// what the withdrawal issue says after its ini block, for people
xr_string WithdrawalDetails(const SInput& in)
{
	const EditorMod::SManifest& m = in.manifest;
	return "**" + (m.name.empty() ? m.id : m.name) + "**" + (m.author.empty() ? xr_string() : " by " + m.author) + " - `" +
		m.id + "`\n\nThe author withdraws the module from the Mod Browser. Installed copies keep working, and submitting the "
		"module again lists it again.\n";
}

// Only the account that owns the listed repository may withdraw a module: the editor refuses
// anyone else before anything is sent, and the catalog checks again by itself.
bool RunWithdraw(const SInput& in, SStanding& standing, xr_string& err)
{
	EditorGithub::SCatalogWithdrawal rq;
	rq.module_id	= in.manifest.id;
	rq.details		= WithdrawalDetails(in);
	rq.token		= in.token;
	const bool ok = EditorGithub::CatalogWithdraw(rq, s_Cancel, [](LPCSTR text) { Stage(text); }, standing.withdrawal, err);
	EditorGithub::WipeString(rq.token);
	if (!ok) return false;
	{
		std::lock_guard<std::mutex> guard(s_Lock);
		s_View.login = standing.withdrawal.login;
	}
	Stage("Reading the submission");
	standing.submission_read = EditorGithub::CatalogIssue(in.manifest.id, in.token, s_Cancel, standing.submission, standing.submission_error);
	ReadListingNow(in, standing);
	return true;
}

void Worker(EJob job, SInput in)
{
	SCheck check;
	SChecked seen;
	SStanding standing;
	bool ok = true;
	xr_string err;
	switch (job)
	{
	case jbCheck:		RunCheck(in, true, check, seen, standing); break;
	case jbVt:			RunVt(in, seen.vt, err); ok = err.empty(); break;
	case jbSubmit:		ok = RunSubmit(in, check, seen, standing, err); break;
	case jbSubmission:	ok = RunStanding(in, standing, err); break;
	case jbWithdraw:	ok = RunWithdraw(in, standing, err); break;
	}
	const bool cancelled = s_Cancel.Cancelled();
	std::lock_guard<std::mutex> guard(s_Lock);
	s_View.state = cancelled ? stCancelled : ok ? stDone : stFailed;
	s_View.error = cancelled ? xr_string("cancelled") : ok ? xr_string() : err;
	s_View.stage.clear();
	if (check.done)		s_View.check = check;
	if (seen.vt.state != "none" && seen.vt.state != "cancelled")
	{
		s_View.virustotal = seen.vt;
		s_View.vt_scanned = jbCheck != job;
	}
	// what the catalog and GitHub said about the module, once a job asked them
	const bool asked = jbSubmission == job || jbWithdraw == job ? ok : (jbCheck == job || jbSubmit == job) && check.done;
	if (asked && !cancelled)
	{
		// a submission GitHub did not name is unknown, never "not submitted"
		s_View.submission_read = in.signed_in && standing.submission_read;
		s_View.submission = s_View.submission_read ? standing.submission : EditorGithub::SCatalogIssue();
		s_View.submission_error = standing.submission_error;
		s_View.withdrawal = in.signed_in ? standing.withdrawal : EditorGithub::SCatalogIssue();
		s_View.listing = standing.listing;
	}
}

void Reap()
{
	if (Running()) return;
	if (s_Thread.joinable()) s_Thread.join();
}

// results belong to one project and one module: another one starts from nothing
void DropStale()
{
	if (Running()) return;
	EditorMod::SManifest m;
	const xr_string root = EditorProject::Active() ? EditorProject::Root() : "";
	if (!root.empty()) EditorMod::Load(root.c_str(), m);
	std::lock_guard<std::mutex> guard(s_Lock);
	if (s_View.root == root && s_View.module_id == m.id) return;
	s_View = SView();
	s_View.root = root;
	s_View.module_id = m.id;
}

// Where the GitHub account comes from, read when the window opens, when a job starts and - for
// MCP - at most every few seconds: the GitHub CLI answers by starting a process.
EditorGithub::EAccountSource	s_AccountSource	= EditorGithub::asNone;
u32								s_AccountRead	= 0;

void NoteAccount(EditorGithub::EAccountSource source)
{
	s_AccountSource = source;
	s_AccountRead = ::GetTickCount();
}

EditorGithub::EAccountSource AccountSource(bool refresh)
{
	if (refresh || !s_AccountRead || ::GetTickCount() - s_AccountRead > 10000)
	{
		EditorGithub::SAccount account;
		EditorGithub::ResolveAccount(account);
		NoteAccount(account.source);
	}
	return s_AccountSource;
}

// a row of the Task Center while a job runs: the window's own footer says the same while the window is up
void JobRow(SProgressTaskInfoVec& out)
{
	EJob job;
	int step;
	xr_string stage, module_id;
	{
		std::lock_guard<std::mutex> guard(s_Lock);
		if (stRunning != s_View.state) return;
		job			= s_View.job;
		step		= s_View.step;
		stage		= s_View.stage;
		module_id	= s_View.module_id;
	}
	static const LPCSTR kTitles[] = { "Checking the Mod Browser Requirements", "Scanning on VirusTotal", "Submitting to the Mod Browser",
		"Reading the Submission", "Withdrawing from the Mod Browser" };
	static const LPCSTR kStops[] = { "Stop Checking", "Stop Scanning", "Stop Submitting", "Stop Refreshing", "Stop Withdrawing" };
	const u32 which = jbVt == job ? 1 : jbSubmit == job ? 2 : jbSubmission == job ? 3 : jbWithdraw == job ? 4 : 0;
	SProgressTaskInfo task;
	task.key		= "catalog";
	task.text		= kTitles[which];
	task.subtitle	= module_id;
	task.detail		= stage.empty() ? xr_string("Working") : stage;
	if (jbSubmit == job)
	{
		for (LPCSTR name : { "Check", "Scan", "Send" })
			task.steps.push_back(name);
		task.step = step;
	}
	task.stop_label	= kStops[which];
	task.stop_tip	= "Stops the job at its next step: nothing more is sent. What already reached GitHub or VirusTotal stays there.";
	task.from_mcp	= !s_FromUI;
	task.elapsed_ms	= ::GetTickCount() - s_JobStarted;
	task.cancelable	= true;
	task.cancel		= [] { s_Cancel.Cancel(); };
	task.cancel_requested = s_Cancel.Cancelled();
	out.push_back(std::move(task));
}

bool Start(EJob job, xr_string& err, bool from_ui = false)
{
	err.clear();
	Reap();
	if (Running())					{ err = "a Mod Browser job is running already"; return false; }
	if (!EditorProject::Active())	{ err = "no active project"; return false; }
	DropStale();

	SInput in;
	in.root = EditorProject::Root();
	if (!EditorMod::Load(in.root.c_str(), in.manifest))			{ err = "mod.ltx not found in the project"; return false; }
	if (!EditorMod::ValidateId(in.manifest.id.c_str()))			{ err = "the module id is missing or invalid (Mod > Mod Settings)"; return false; }
	in.key = EditorCatalog::KeyState();
	EditorGithub::SAccount account;
	EditorGithub::ResolveAccount(account);
	NoteAccount(account.source);
	in.signed_in = EditorGithub::asNone != account.source;
	in.token = account.token;
	if ((jbSubmit == job || jbSubmission == job || jbWithdraw == job) && !in.signed_in)
		{ err = "not signed in to GitHub: sign in from Mod > Publish Release (MCP: mod_publish action=sign_in or save_token)"; return false; }
	// a withdrawal needs neither key: only the account that owns the listing counts
	if (jbSubmission != job && jbWithdraw != job) EditorGithub::ReadVtKey(in.vt_key);
	if ((jbVt == job || jbSubmit == job) && in.vt_key.empty())
	{
		err = "no VirusTotal key: the catalog lists a package only when VirusTotal knows it, and the packages are scanned "
			  "with your own key - a free account at virustotal.com has one (MCP: mod_catalog action=vt_key)";
		return false;
	}
	{
		std::lock_guard<std::mutex> guard(s_Lock);
		s_View.job		= job;
		s_View.state	= stRunning;
		s_View.stage	= "Starting";
		s_View.step		= 0;
		s_View.error.clear();
	}
	s_Cancel.Reset();
	s_FromUI		= from_ui;
	s_JobStarted	= ::GetTickCount();
	UIProgressCenter::RegisterSource("catalog_job", &JobRow);
	s_Thread = std::thread(&Worker, job, in);
	return true;
}

//------------------------------------------------------------------------------
// json
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

void JsonField(xr_string& out, LPCSTR key, LPCSTR value, bool first = false)
{
	out += first ? "\"" : ",\"";
	out += key;
	out += "\":\"";
	JsonAppend(out, value);
	out += "\"";
}

LPCSTR JsonBool(bool v) { return v ? "true" : "false"; }

bool HasVtKey()
{
	xr_string key;
	const bool has = EditorGithub::ReadVtKey(key);
	EditorGithub::WipeString(key);
	return has;
}

// an issue's fields, left open for the caller to add its own and close
void IssueJson(xr_string& out, const EditorGithub::SCatalogIssue& issue)
{
	out += NqUtil::Format("{\"number\":%llu", issue.number);
	JsonField(out, "state", issue.state.c_str());
	JsonField(out, "verdict", issue.verdict.c_str());
	JsonField(out, "verdict_text", issue.verdict_text.c_str());
	JsonField(out, "url", issue.url.c_str());
	JsonField(out, "note", issue.note.c_str());
}

void StatusJson(xr_string& out)
{
	Reap();
	DropStale();
	const SView v = Snapshot();
	const EditorCatalog::SKeyState key = EditorCatalog::KeyState();
	out = "{\"ok\":true";
	JsonField(out, "catalog", EditorGithub::CatalogRepo().c_str());
	out += NqUtil::Format(",\"key\":{\"exists\":%s,\"locked\":%s", JsonBool(key.exists), JsonBool(key.locked));
	JsonField(out, "path", key.path.c_str());
	JsonField(out, "key_id", key.key_id.c_str());
	JsonField(out, "public", key.public_key.c_str());
	JsonField(out, "error", key.error.c_str());
	out += NqUtil::Format("},\"vt_key\":%s", JsonBool(HasVtKey()));
	const EditorGithub::EAccountSource source = AccountSource(false);
	out += NqUtil::Format(",\"account\":{\"signed_in\":%s", JsonBool(EditorGithub::asNone != source));
	JsonField(out, "source", EditorGithub::SourceText(source));
	JsonField(out, "login", v.login.c_str());
	out += "},\"job\":{";
	JsonField(out, "kind", JobName(v.job), true);
	JsonField(out, "state", StateName(v.state));
	out += NqUtil::Format(",\"pending\":%s", JsonBool(stRunning == v.state));
	JsonField(out, "stage", v.stage.c_str());
	JsonField(out, "error", v.error.c_str());
	out += NqUtil::Format("},\"check\":{\"done\":%s,\"blocking\":%s,\"release\":{", JsonBool(v.check.done), JsonBool(v.check.blocking));
	JsonField(out, "version", v.check.version.c_str(), true);
	JsonField(out, "url", v.check.url.c_str());
	out += NqUtil::Format(",\"signed\":%s", JsonBool(v.check.signed_));
	JsonField(out, "key_id", v.check.key_id.c_str());
	out += "},\"findings\":[";
	for (u32 i = 0; i < v.check.findings.size(); ++i)
	{
		out += i ? ",{" : "{";
		JsonField(out, "level", EditorCatalog::LevelName(v.check.findings[i].level), true);
		JsonField(out, "code", v.check.findings[i].code.c_str());
		JsonField(out, "text", v.check.findings[i].text.c_str());
		out += "}";
	}
	out += "]},\"virustotal\":";
	EditorVirusTotal::Json(out, v.virustotal);
	out += ",\"submission\":";
	IssueJson(out, v.submission);
	out += NqUtil::Format(",\"known\":%s,\"listed\":%s},\"withdrawal\":", JsonBool(v.submission_read), JsonBool(v.Listed()));
	IssueJson(out, v.withdrawal);
	out += "},\"listing\":{";
	JsonField(out, "state", v.listing.state.c_str(), true);
	JsonField(out, "github", v.listing.github.c_str());
	out += NqUtil::Format(",\"this_key\":%s},\"window\":%s}", JsonBool(v.listing.this_key), JsonBool(s_Want));
}

void ErrorJson(xr_string& out, const xr_string& err)
{
	out = "{\"ok\":false,\"error\":\"";
	JsonAppend(out, err.c_str());
	out += "\"}";
}

//------------------------------------------------------------------------------
// the window: an L dialog of cards whose height is fixed once it opens - what the checks find scrolls
// inside it instead of growing it (docs/UI_DESIGN.md, section 6)
//------------------------------------------------------------------------------
const char* kTitle			= EditorUiContract::kDialogSubmit;
const char* kTitleWithdraw	= EditorUiContract::kDialogCatalogWithdraw;
const float kBodyPt			= 600.f;	// the body's height while the work area has room for it

EditorMod::SManifest		s_Manifest;					// what the window names
u32							s_ManifestGen	= 0;
EditorCatalog::SKeyState	s_Key;						// read on opening and after every key action
bool						s_KeyCreated	= false;	// this session made it: the copy note stands out
xr_string					s_KeyTarget;				// where Create Key writes
xr_string					s_KeyError;
char						s_Password[128]	= {};
char						s_Repeat[128]	= {};
char						s_VtInput[160]	= {};
bool						s_VtKnown		= false;
xr_string					s_VtError;
bool						s_PickQueued	= false;
xr_string					s_StartError;				// why a job the window asked for did not start
bool						s_WantWithdraw	= false;	// the withdrawal's question is up

void StartFromUI(EJob job)
{
	xr_string err;
	s_StartError = Start(job, err, true) ? xr_string() : err;
}

ImU32 Token(GrizzlyTheme::ColorToken token, float alpha = 1.f)
{
	return GrizzlyTheme::U32(token, alpha);
}

void WipeInputs()
{
	::SecureZeroMemory(s_Password, sizeof(s_Password));
	::SecureZeroMemory(s_Repeat, sizeof(s_Repeat));
	::SecureZeroMemory(s_VtInput, sizeof(s_VtInput));
}

void RefreshKey()
{
	s_Key = EditorCatalog::KeyState();
	if (s_KeyTarget.empty() || s_Key.exists) s_KeyTarget = s_Key.path;
}

void RefreshManifest()
{
	if (s_ManifestGen == EditorMod::ManifestGeneration() && !s_Manifest.id.empty()) return;
	s_ManifestGen = EditorMod::ManifestGeneration();
	s_Manifest = EditorMod::SManifest();
	EditorMod::Load(EditorProject::Root(), s_Manifest);
}

// A native file dialog, outside the ImGui pass. `save` asks for a new file.
bool PickFile(bool save, LPCWSTR title, const xr_string& start, xr_string& picked)
{
	picked.clear();
	const HRESULT co = ::CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
	IFileDialog* dlg = 0;
	if (SUCCEEDED(::CoCreateInstance(save ? CLSID_FileSaveDialog : CLSID_FileOpenDialog, NULL, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&dlg))))
	{
		const COMDLG_FILTERSPEC types[] = { { L"Grizzly author key (*.xkey)", L"*.xkey" }, { L"All files", L"*.*" } };
		dlg->SetFileTypes(2, types);
		dlg->SetDefaultExtension(L"xkey");
		dlg->SetTitle(title);
		DWORD opts = 0;
		dlg->GetOptions(&opts);
		// a key file is never replaced (CreateKey refuses), so the dialog does not offer to
		dlg->SetOptions(save ? (opts & ~FOS_OVERWRITEPROMPT) | FOS_FORCEFILESYSTEM : opts | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST);
		if (!start.empty())
		{
			wchar_t wide[MAX_PATH] = {};
			::MultiByteToWideChar(CP_UTF8, 0, start.c_str(), -1, wide, MAX_PATH);
			wchar_t* slash = wcsrchr(wide, L'\\');
			if (slash)
			{
				if (save) dlg->SetFileName(slash + 1);
				*slash = 0;
				IShellItem* folder = 0;
				if (SUCCEEDED(::SHCreateItemFromParsingName(wide, NULL, IID_PPV_ARGS(&folder))))
				{
					dlg->SetFolder(folder);
					folder->Release();
				}
			}
		}
		IShellItem* item = 0;
		if (SUCCEEDED(dlg->Show(EDevice ? EDevice->m_hWnd : NULL)) && SUCCEEDED(dlg->GetResult(&item)))
		{
			PWSTR w = 0;
			if (SUCCEEDED(item->GetDisplayName(SIGDN_FILESYSPATH, &w)))
			{
				char path[MAX_PATH * 3] = {};
				::WideCharToMultiByte(CP_UTF8, 0, w, -1, path, sizeof(path), 0, 0);
				picked = path;
				::CoTaskMemFree(w);
			}
			item->Release();
		}
		dlg->Release();
	}
	if (co == S_OK) ::CoUninitialize();
	return !picked.empty();
}

void PickKeyTarget()
{
	s_PickQueued = false;
	xr_string picked;
	if (s_Want && PickFile(true, L"Where the new author key goes", s_KeyTarget, picked))
		s_KeyTarget = picked;
}

void PickKeyFile()
{
	s_PickQueued = false;
	xr_string picked;
	if (!s_Want || !PickFile(false, L"Use an author key file", s_Key.path, picked)) return;
	if (!EditorCatalog::UseKeyFile(picked.c_str(), s_KeyError)) return;
	s_KeyError.clear();
	s_KeyCreated = false;
	RefreshKey();
}

// required is never grey: a requirement the submission still has to meet reads in its own tone
XUI::Tone LevelTone(ELevel level)
{
	switch (level)
	{
	case lvOk:			return XUI::Tone::Success;
	case lvReview:		return XUI::Tone::Warning;
	case lvRequired:	return XUI::Tone::Required;
	case lvBlock:		return XUI::Tone::Error;
	}
	return XUI::Tone::Muted;
}

// the width a kit button of this label takes outside a dialog's footer
float KitButtonWidth(LPCSTR label)
{
	return ImGui::CalcTextSize(label, nullptr, true).x + XUI::Pt(20.f);
}

// the next item, `width` wide, ends at the right edge of the current line
void SameLineRight(float width)
{
	ImGui::SameLine();
	const float x = ImGui::GetCursorPosX() + ImGui::GetContentRegionAvail().x - width;
	ImGui::SameLine(_max(ImGui::GetCursorPosX(), x));
}

// a path in a property row's value: its end stays, the start is what the ellipsis drops
void PathValue(const xr_string& path)
{
	const ImGuiStyle& style = ImGui::GetStyle();
	const float width = ImGui::CalcItemWidth();
	ImGui::SetCursorPosX(ImGui::GetCursorPosX() + style.FramePadding.x);
	ImGui::AlignTextToFramePadding();
	ImGui::PushStyleColor(ImGuiCol_Text, Token(GrizzlyTheme::ColorToken::TextSecondary));
	XUI::TextEllipsis(path.c_str(), _max(XUI::Em(2.f), width - style.FramePadding.x * 2.f), XUI::Clip::Start);
	ImGui::PopStyleColor();
}

// a password field and its button on one row, the field taking what the button leaves
bool PasswordAndButton(LPCSTR id, LPCSTR hint, char* buf, size_t size, LPCSTR button, bool on, LPCSTR why)
{
	ImGui::SetNextItemWidth(_max(XUI::Em(6.f), ImGui::GetContentRegionAvail().x - KitButtonWidth(button) - ImGui::GetStyle().ItemSpacing.x));
	const bool enter = ImGui::InputTextWithHint(id, hint, buf, size, ImGuiInputTextFlags_Password | ImGuiInputTextFlags_EnterReturnsTrue);
	ImGui::SameLine();
	return XUI::Button(button, XUI::Kind::Secondary, on, why) || (enter && on);
}

void DrawKeyCard()
{
	const bool readable = s_Key.exists && s_Key.error.empty();
	const XUI::Tone tone = !readable ? XUI::Tone::Error : s_Key.locked ? XUI::Tone::Required : XUI::Tone::Success;
	XUI::BeginCard("##card_key", "Author Key", tone, !readable || s_Key.locked);
	if (readable)
	{
		ImGui::SameLine();
		XUI::Badge(s_Key.locked ? "Locked" : "Ready", tone);
	}
	if (!s_Key.exists)
	{
		XUI::StatusText(XUI::Tone::Muted, "No author key yet. It signs every release of the module, and the Mod Browser lists signed releases "
			"only. It is a file you keep - never part of the project.");
		const bool differ = 0 != strcmp(s_Password, s_Repeat);
		if (XUI::BeginPropertyGrid("catalog_key"))
		{
			if (XUI::PropertyRow("Key File", "Where Create Key writes the new key file. It never replaces a file that is there."))
			{
				PathValue(s_KeyTarget);
				if (!s_PickQueued && XUI::PropertyTrailing("##key_target", XUI::Icon::Open, "Choose where the new key goes"))
					s_PickQueued = UI->DeferUIWork(&PickKeyTarget);
			}
			if (XUI::PropertyRow("Password", "Optional. Protects the file if it ever leaves your hands; the editor asks for it once per session. "
					"Forgotten, it cannot be recovered - and neither can the key."))
				ImGui::InputTextWithHint("##password", "Optional", s_Password, sizeof(s_Password), ImGuiInputTextFlags_Password);
			if (XUI::PropertyRow("Repeat", "The password once more.", differ && s_Repeat[0] ? XUI::PropInvalid : 0))
			{
				ImGui::InputTextWithHint("##repeat", "Repeat the password", s_Repeat, sizeof(s_Repeat), ImGuiInputTextFlags_Password);
				if (differ && s_Repeat[0]) XUI::PropertyMessage(XUI::Tone::Error, "The passwords differ.");
			}
			XUI::EndPropertyGrid();
		}
		if (XUI::Button("Create Key", XUI::Kind::Secondary, !differ, "The two passwords differ."))
		{
			if (EditorCatalog::CreateKey(s_KeyTarget.c_str(), s_Password, s_KeyError))
			{
				s_KeyError.clear();
				s_KeyCreated = true;
			}
			WipeInputs();
			RefreshKey();
		}
		ImGui::SameLine();
		if (XUI::Button("Use Key File...", XUI::Kind::Secondary, !s_PickQueued, "A file dialog is open."))
			s_PickQueued = UI->DeferUIWork(&PickKeyFile);
		XUI::Tip("The key file from another computer, or its copy.");
	}
	else if (!s_Key.error.empty())
	{
		XUI::StatusText(XUI::Tone::Error, ("The key file does not read: " + s_Key.error + ".").c_str());
		if (XUI::BeginPropertyGrid("catalog_key"))
		{
			if (XUI::PropertyRow("Key File", "The key file the editor remembers.", XUI::PropReadOnly))
				PathValue(s_Key.path);
			XUI::EndPropertyGrid();
		}
		if (XUI::Button("Use Another Key File...", XUI::Kind::Secondary, !s_PickQueued, "A file dialog is open."))
			s_PickQueued = UI->DeferUIWork(&PickKeyFile);
	}
	else if (s_Key.locked || s_KeyCreated || XUI::Section("More##key", false))
	{
		if (XUI::BeginPropertyGrid("catalog_key"))
		{
			if (XUI::PropertyRow("Key", "The key's id: what a signed release names.", XUI::PropReadOnly))
				XUI::PropertyText(s_Key.key_id.c_str());
			if (XUI::PropertyRow("Key File", "The key file this computer signs with.", XUI::PropReadOnly))
				PathValue(s_Key.path);
			if (s_Key.locked && XUI::PropertyRow("Password", "The key file is protected: its password opens it until the editor closes.",
					XUI::PropRequired))
			{
				if (PasswordAndButton("##unlock", "Key password", s_Password, sizeof(s_Password), "Unlock", s_Password[0] != 0,
						"Type the key file's password first."))
				{
					if (EditorCatalog::Unlock(s_Password, s_KeyError)) s_KeyError.clear();
					WipeInputs();
					RefreshKey();
				}
			}
			XUI::EndPropertyGrid();
		}
		XUI::StatusText(s_KeyCreated ? XUI::Tone::Warning : XUI::Tone::Muted, "Keep a copy of this file somewhere safe - a USB stick, a "
			"password manager. Without it no update of this module can be signed.");
		// the card's actions, named and framed like every other button of the window
		if (XUI::Button("Use Another Key File...", XUI::Kind::Secondary, !s_PickQueued, "A file dialog is open."))
			s_PickQueued = UI->DeferUIWork(&PickKeyFile);
		if (!s_PickQueued)
			XUI::Tip("Signs with another key file from now on: the key file from another computer, or its copy.");
		ImGui::SameLine();
		if (XUI::Button("Show in Folder", XUI::Kind::Secondary))
			::ShellExecuteA(0, "open", "explorer.exe", ("/select,\"" + s_Key.path + "\"").c_str(), 0, SW_SHOWNORMAL);
		XUI::Tip("Opens the key file's folder in Explorer, the file selected - to copy it somewhere safe.");
		if (!s_Key.locked && !s_KeyCreated) XUI::EndSection();
	}
	if (!s_KeyError.empty())
		XUI::StatusText(XUI::Tone::Error, s_KeyError.c_str());
	XUI::EndCard();
}

// What section 7 asks, failures first; the requirements that are met fold away - a wall of green reads as
// nothing. The VirusTotal findings are the VirusTotal card's: its verdict appears there alone.
void DrawRequirementsCard(const SView& view)
{
	xr_vector<const SFinding*> open, met;
	for (const SFinding& f : view.check.findings)
	{
		if (f.code == "virustotal") continue;
		(lvOk == f.level ? met : open).push_back(&f);
	}
	std::stable_sort(open.begin(), open.end(), [](const SFinding* a, const SFinding* b) { return a->level > b->level; });
	u32 blocks = 0, required = 0;
	for (const SFinding* f : open)
	{
		blocks += lvBlock == f->level ? 1 : 0;
		required += lvRequired == f->level ? 1 : 0;
	}
	const bool running = stRunning == view.state;
	const bool checking = running && (jbCheck == view.job || jbSubmit == view.job);
	const XUI::Tone tone = blocks ? XUI::Tone::Error : required ? XUI::Tone::Required : view.check.done ? XUI::Tone::Success : XUI::Tone::Muted;
	XUI::BeginCard("##card_requirements", "Requirements", tone, blocks > 0);
	if (view.check.done && !checking)
	{
		ImGui::SameLine();
		XUI::Badge(blocks ? NqUtil::Format("%u to fix", blocks).c_str() : "Met", blocks ? XUI::Tone::Error : XUI::Tone::Success);
	}
	SameLineRight(KitButtonWidth("Check Again"));
	if (XUI::Button("Check Again", XUI::Kind::Secondary, !running, "A Mod Browser job is running."))
		StartFromUI(jbCheck);
	if (!running)
		XUI::Tip("Reads the latest release the way the game does and checks what the Mod Browser requires.");

	if (checking)
		XUI::StatusText(XUI::Tone::Muted, "Checking...");
	else if (!view.check.done)
		XUI::StatusText(XUI::Tone::Muted, "Not checked yet.");
	for (const SFinding* f : open)
		XUI::StatusText(LevelTone(f->level), f->text.c_str());
	if (view.check.done && open.empty() && !checking)
		XUI::StatusText(XUI::Tone::Success, "Every requirement is met.");
	if (!met.empty())
	{
		if (XUI::Section(NqUtil::Format("Met (%u)###met", (u32)met.size()).c_str(), false))
		{
			for (const SFinding* f : met)
				XUI::StatusText(XUI::Tone::Success, f->text.c_str());
			XUI::EndSection();
		}
	}
	if (!s_StartError.empty())
		XUI::StatusText(XUI::Tone::Error, s_StartError.c_str());
	XUI::EndCard();
}

// the one place the VirusTotal verdict appears; required, so its open state is never grey
void DrawVirusTotalCard(const SView& view)
{
	const bool settled = VtSettled(view.virustotal);
	const bool running = stRunning == view.state;
	const bool scanning = running && (jbVt == view.job || jbSubmit == view.job);
	const XUI::Tone tone = s_VtKnown && !view.virustotal.packages.empty() ? LevelTone(VtLevel(view.virustotal.state, view.vt_scanned)) :
		XUI::Tone::Required;
	XUI::BeginCard("##card_vt", "VirusTotal (required)", tone, !settled, !settled);
	if (!s_VtKnown)
	{
		XUI::StatusText(XUI::Tone::Required, "Required: the catalog lists a package only when VirusTotal knows it, and every release is "
			"scanned with your own VirusTotal key. A free account at virustotal.com has one.");
		if (PasswordAndButton("##vt_key", "VirusTotal API key", s_VtInput, sizeof(s_VtInput), "Save Key", s_VtInput[0] != 0,
				"Paste the key first."))
		{
			if (EditorGithub::SaveVtKey(s_VtInput, s_VtError)) s_VtError.clear();
			WipeInputs();
			s_VtKnown = HasVtKey();
		}
		XUI::Tip("Kept in the Windows Credential Manager - never in the project.");
		if (!s_VtError.empty())
			XUI::StatusText(XUI::Tone::Error, s_VtError.c_str());
		XUI::EndCard();
		return;
	}
	// a check looks the packages up: until it has, "not scanned" would be a guess
	const bool looking = running && jbCheck == view.job && view.virustotal.packages.empty();
	if (scanning)
		XUI::StatusText(XUI::Tone::Muted, "Scanning...");
	else if (looking)
		XUI::StatusText(XUI::Tone::Muted, "Looking the packages up...");
	// the requirement stays in sight until every package has a report
	if (view.virustotal.packages.empty() && !looking)
		XUI::StatusText(XUI::Tone::Required, "Not scanned yet. Every package needs a VirusTotal report before the module is listed: Scan "
			"runs it now, and Submit scans first.");
	else if (!view.virustotal.packages.empty() && !settled)
		XUI::StatusText(XUI::Tone::Required, "Not every package has a VirusTotal report yet: the module is not submitted without one.");
	u32 reports = 0;
	for (const EditorVirusTotal::SResult& r : view.virustotal.packages)
	{
		XUI::StatusText(LevelTone(VtLevel(r.state, view.vt_scanned)), (EditorVirusTotal::Describe(r) + ".").c_str());
		if (VtHasReport(r))
			++reports;
	}
	if (XUI::Button("Scan", XUI::Kind::Secondary, !running, "A Mod Browser job is running."))
	{
		xr_string err;
		s_VtError = Start(jbVt, err, true) ? xr_string() : err;
	}
	if (!running)
		XUI::Tip("Looks every package of the latest release up on VirusTotal, uploads one it does not know and waits for its report - a "
			"few minutes. VirusTotal shares uploaded files with the security industry.");
	if (!view.virustotal.packages.empty())
	{
		static const LPCSTR kReportLabels[] = { "Open Report", "Open Reports" };
		ImGui::SameLine();
		if (XUI::Button(reports > 1 ? kReportLabels[1] : kReportLabels[0], XUI::Kind::Secondary, reports > 0,
				"No package has a finished VirusTotal report yet.", kReportLabels, 2))
		{
			for (const EditorVirusTotal::SResult& r : view.virustotal.packages)
				if (VtHasReport(r))
					::ShellExecuteA(0, "open", EditorGithub::VtReportPage(r.package.sha256).c_str(), 0, 0, SW_SHOWNORMAL);
		}
		if (reports)
			XUI::Tip(reports > 1 ? "Opens the VirusTotal report of every package in the browser." :
				"Opens the package's VirusTotal report in the browser.");
	}
	ImGui::SameLine();
	if (XUI::Button("Forget VirusTotal Key", XUI::Kind::Secondary))
	{
		EditorGithub::ForgetVtKey();
		s_VtKnown = HasVtKey();
	}
	XUI::Tip("Removes the VirusTotal API key the editor saved in the Windows Credential Manager. A key from VT_API_KEY stays in "
		"effect. The author key is not touched.");
	if (!s_VtError.empty())
		XUI::StatusText(XUI::Tone::Error, s_VtError.c_str());
	XUI::EndCard();
}

// the tone of a card about an issue of the catalog: open, accepted, refused, or merely closed
XUI::Tone IssueTone(const EditorGithub::SCatalogIssue& issue)
{
	if (!issue.number) return XUI::Tone::Muted;
	if (issue.state == "open") return XUI::Tone::Info;
	return issue.verdict == "accepted" ? XUI::Tone::Success : issue.verdict == "refused" ? XUI::Tone::Error : XUI::Tone::Muted;
}

// what an issue of the catalog says now
void IssueLines(const EditorGithub::SCatalogIssue& issue, bool done_state, LPCSTR accepted_done, LPCSTR accepted_soon)
{
	// Open, the catalog's newest verdict says what it waits for. Closed, only an accepted or a refused one says
	// what became of it: the author may have closed it after a waiting one.
	xr_string text, verdict;
	ELevel level = lvNote, verdict_level = lvNote;
	if (issue.state == "open")
	{
		text = NqUtil::Format("#%llu is open: the catalog checks it.", issue.number);
		verdict = issue.verdict_text;
		verdict_level = issue.verdict == "accepted" ? lvOk : issue.verdict == "refused" ? lvBlock : lvReview;
	}
	else if (issue.verdict == "accepted")
	{
		level = lvOk;
		text = NqUtil::Format(done_state ? accepted_done : accepted_soon, issue.number);
	}
	else if (issue.verdict == "refused")
	{
		level = verdict_level = lvBlock;
		text = NqUtil::Format("#%llu was refused.", issue.number);
		verdict = issue.verdict_text;
	}
	else
		text = NqUtil::Format("#%llu was closed.", issue.number);
	XUI::StatusText(LevelTone(level), text.c_str());
	if (!verdict.empty())
		XUI::StatusText(LevelTone(verdict_level), verdict.c_str());
}

void DrawSubmissionCard(const SView& view)
{
	const EditorGithub::SCatalogIssue& submission = view.submission;
	const bool signed_in = EditorGithub::asNone != s_AccountSource;
	const XUI::Tone tone = IssueTone(submission);
	XUI::BeginCard("##card_submission", "Submission", tone, XUI::Tone::Error == tone);
	// "not submitted" only once GitHub said so: every job but the scan looks the submission up; signed out, the
	// line below says what is missing
	if (!view.submission_read)
	{
		if (signed_in && stRunning == view.state && jbVt != view.job)
			XUI::StatusText(XUI::Tone::Muted, "Looking up the submission...");
		else if (signed_in && !view.submission_error.empty())
			XUI::StatusText(XUI::Tone::Warning, ("The submission could not be looked up: " + Reason(view.submission_error) + ".").c_str());
		else if (signed_in)
			XUI::StatusText(XUI::Tone::Muted, "Not looked up yet: Check Again reads where it stands.");
	}
	else if (!submission.number)
		XUI::StatusText(XUI::Tone::Muted, "Not submitted yet.");
	else
	{
		IssueLines(submission, view.Listed(), "#%llu was accepted: the module is in the Mod Browser.",
			"#%llu was accepted: the module appears in the Mod Browser within minutes.");
		if (!submission.note.empty())
			XUI::StatusText(XUI::Tone::Muted, submission.note.c_str());
		if (XUI::Button("Open Submission", XUI::Kind::Secondary))
			::ShellExecuteA(0, "open", submission.url.c_str(), 0, 0, SW_SHOWNORMAL);
	}
	const xr_string where = "Submitting opens a public issue at github.com/" + EditorGithub::CatalogRepo() +
		(view.login.empty() ? xr_string() : " from " + view.login) + ".";
	XUI::StatusText(XUI::Tone::Muted, signed_in ? where.c_str() : "Submitting needs a GitHub account: sign in from Publish Release.");
	if (stFailed == view.state && !view.error.empty() && jbWithdraw != view.job)
		XUI::StatusText(XUI::Tone::Error, view.error.c_str());
	XUI::EndCard();
}

// A listed module can be withdrawn by the account it was listed from; a withdrawal already sent is
// followed like the submission.
void DrawWithdrawalCard(const SView& view)
{
	const EditorGithub::SCatalogIssue& w = view.withdrawal;
	const bool listed = view.listing.state == "listed", withdrawn = view.listing.state == "withdrawn";
	const bool failed = stFailed == view.state && jbWithdraw == view.job && !view.error.empty();
	if (!listed && !withdrawn && !w.number && w.note.empty() && !failed) return;
	XUI::BeginCard("##card_withdrawal", "Withdrawal", failed ? XUI::Tone::Error : IssueTone(w), failed);
	if (stRunning == view.state && jbWithdraw == view.job)
		XUI::StatusText(XUI::Tone::Muted, "Withdrawing...");
	if (w.number)
		IssueLines(w, withdrawn, "#%llu was accepted: the module is withdrawn from the Mod Browser.",
			"#%llu was accepted: the module leaves the Mod Browser within minutes.");
	if (!w.note.empty())
		XUI::StatusText(XUI::Tone::Muted, w.note.c_str());
	if (withdrawn && w.number && w.verdict == "accepted")
		XUI::StatusText(XUI::Tone::Muted, "Submitting the module again lists it again.");
	else if (withdrawn)
		XUI::StatusText(XUI::Tone::Muted, "Not in the Mod Browser: its author withdrew it. Submitting lists it again.");
	if (listed)
		XUI::StatusText(XUI::Tone::Muted, "Withdrawing takes the module out of the Mod Browser: it is no longer offered, and players who "
			"installed it there get no more updates. Installed copies keep working.");

	// only the account the listing came from may ask; the catalog checks it again by itself
	const xr_string owner = Owner(view.listing.github);
	const bool other_account = listed && !view.login.empty() && 0 != _stricmp(owner.c_str(), view.login.c_str());
	if (other_account)
		XUI::StatusText(XUI::Tone::Warning, ("Only " + owner + ", the owner of " + view.listing.github + ", can withdraw it.").c_str());
	if (listed)
	{
		const bool can = stRunning != view.state && EditorGithub::asNone != s_AccountSource && !other_account;
		// the button that opens the question is quiet; the one that acts, inside the question, is the Danger one
		if (XUI::Button("Withdraw...", XUI::Kind::GhostDanger, can, other_account ? "Only the account the module was listed from can withdraw it." :
				stRunning == view.state ? "A Mod Browser job is running." : "Sign in to GitHub first: Publish Release."))
			s_WantWithdraw = true;
		if (can)
			XUI::Tip("Asks the catalog to withdraw the module: a public issue in the catalog repository, judged by the catalog's own checks. "
				"The id stays bound to your author key. Asks first.");
		if (w.number) ImGui::SameLine();
	}
	if (w.number && XUI::Button("Open Withdrawal", XUI::Kind::Secondary))
		::ShellExecuteA(0, "open", w.url.c_str(), 0, 0, SW_SHOWNORMAL);
	if (failed)
		XUI::StatusText(XUI::Tone::Error, view.error.c_str());
	XUI::EndCard();
}

// The one question before a withdrawal leaves, as a sheet over the window: what it does, for whom, and that it is
// public. Cancel is the default - Enter and Esc both back out. Drawn whether or not dialogs are suppressed:
// MCP show confirm=withdraw puts it up to be looked at, and only a click in it withdraws anything.
void DrawWithdrawSheet(const SView& view)
{
	if (!XUI::BeginSheet(kTitleWithdraw, &s_WantWithdraw, XUI::DialogSize::M))
		return;
	XUI::BeginDialogBody();
	{
		XUI::FontScope strong(XUI::Font::Strong);
		ImGui::PushTextWrapPos(0.f);
		ImGui::TextUnformatted(NqUtil::Format("Withdraw %s (%s) from the Mod Browser?",
			s_Manifest.name.empty() ? s_Manifest.id.c_str() : s_Manifest.name.c_str(), s_Manifest.id.c_str()).c_str());
		ImGui::PopTextWrapPos();
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
	XUI::StatusText(XUI::Tone::Warning, "The Mod Browser stops offering it, and players who installed it there get no more updates.");
	XUI::StatusText(XUI::Tone::Muted, "Installed copies keep working.");
	XUI::StatusText(XUI::Tone::Muted, "The id stays bound to your author key: submitting the module again lists it again.");
	const xr_string where = "The request is a public issue at github.com/" + EditorGithub::CatalogRepo() +
		(view.login.empty() ? xr_string() : ", from " + view.login) + ", judged by the catalog's own checks.";
	XUI::StatusText(XUI::Tone::Muted, where.c_str());
	if (XUI::BeginDialogFooter())
	{
		const bool signed_in = EditorGithub::asNone != s_AccountSource;
		const bool can = stRunning != view.state && signed_in;
		if (XUI::DialogButton("Withdraw", XUI::Kind::Danger, false, can, signed_in ? "A Mod Browser job is running." :
				"Sign in to GitHub first: Publish Release."))
		{
			StartFromUI(jbWithdraw);
			s_WantWithdraw = false;
		}
		if (XUI::DialogButton("Cancel", XUI::Kind::Secondary, true))
			s_WantWithdraw = false;
	}
	XUI::EndDialog();
}

void DrawWindow()
{
	static bool was_open = false;
	if (was_open && !s_Want)
	{
		WipeInputs();
		s_WantWithdraw = false;
	}
	was_open = s_Want;
	if (!XUI::BeginDialog(kTitle, &s_Want, XUI::DialogSize::L))
		return;
	RefreshManifest();
	const SView view = Snapshot();
	XUI::BeginDialogBody();
	const float top = ImGui::GetCursorPosY();

	// the module, and what the window does for it
	const xr_string name = s_Manifest.name.empty() ? s_Manifest.id : s_Manifest.name;
	{
		XUI::FontScope title(XUI::Font::Title);
		XUI::TextEllipsis(name.c_str(), _max(XUI::Em(6.f), ImGui::GetContentRegionAvail().x - ImGui::CalcTextSize(s_Manifest.id.c_str()).x -
			XUI::Em(2.f)));
	}
	if (!s_Manifest.name.empty() && s_Manifest.name != s_Manifest.id)
	{
		ImGui::SameLine();
		ImGui::PushStyleColor(ImGuiCol_Text, Token(GrizzlyTheme::ColorToken::TextMuted));
		XUI::TextEllipsis(s_Manifest.id.c_str());
		ImGui::PopStyleColor();
	}
	XUI::StatusText(XUI::Tone::Muted, "Lists the module in the Mod Browser of Dead Air: Refined. The catalog judges every submission by "
		"itself and lists the ones that pass its checks.");
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));

	DrawKeyCard();
	DrawRequirementsCard(view);
	DrawVirusTotalCard(view);
	DrawSubmissionCard(view);
	DrawWithdrawalCard(view);

	// a fixed height: while the dialog fits itself to what it holds, the body asks for the room a finished check
	// takes, so the check that ends later scrolls inside instead of growing the window (#8)
	const float room = ImGui::GetWindowHeight() - ImGui::GetStyle().WindowPadding.y * 2.f;
	const float used = ImGui::GetCursorPosY() - top;
	const float want = _min(XUI::Pt(kBodyPt), room);
	if (used < want)
		ImGui::Dummy(ImVec2(0.f, want - used));

	const bool running = stRunning == view.state;
	const bool signed_in = EditorGithub::asNone != s_AccountSource;
	u32 blocks = 0;
	LPCSTR first_block = nullptr;
	for (const SFinding& f : view.check.findings)
		if (lvBlock == f.level)
		{
			if (!blocks) first_block = f.text.c_str();
			++blocks;
		}
	const bool ready = view.check.done && !view.check.blocking && signed_in && !running;
	LPCSTR why = running ? "A Mod Browser job is running." : !signed_in ? "Sign in to GitHub first: Publish Release." :
		!view.check.done ? "Check the requirements first." : first_block ? first_block : "Every requirement above has to be met.";
	const xr_string status = !running && view.check.done && blocks ?
		NqUtil::Format("%u %s to fix", blocks, blocks == 1 ? "requirement" : "requirements") : xr_string();
	if (XUI::BeginDialogFooter(status.empty() ? nullptr : status.c_str(), XUI::Tone::Warning))
	{
		// the stage on the left, in what the buttons leave; the job's Stop is the first button, named after the job
		if (running)
		{
			XUI::FooterBusy(view.stage.empty() ? "Working..." : view.stage.c_str());
			static const LPCSTR kStopLabels[] = { "Stop Checking", "Stop Scanning", "Stop Submitting", "Stop Refreshing",
				"Stop Withdrawing" };
			const u32 which = jbVt == view.job ? 1 : jbSubmit == view.job ? 2 : jbSubmission == view.job ? 3 : jbWithdraw == view.job ? 4 : 0;
			const bool stopping = s_Cancel.Cancelled();
			if (XUI::Button(kStopLabels[which], XUI::Kind::Secondary, !stopping, "Stopping at the next step.", kStopLabels, 5))
				s_Cancel.Cancel();
			if (!stopping)
				XUI::Tip("Stops the job at its next step: nothing more is sent. What already reached GitHub or VirusTotal stays there.");
		}
		// the scan is part of the submission, and the button says so while a report is missing; its width stays
		static const LPCSTR kSubmitLabels[] = { "Scan and Submit", "Scan and Update", "Update Submission", "Submit" };
		const bool update = view.submission.number && view.submission.state == "open";
		const bool scan = !VtSettled(view.virustotal);
		LPCSTR label = update ? (scan ? kSubmitLabels[1] : kSubmitLabels[2]) : (scan ? kSubmitLabels[0] : kSubmitLabels[3]);
		if (XUI::Button(label, XUI::Kind::Primary, ready, why, kSubmitLabels, 4))
			StartFromUI(jbSubmit);
		if (ready)
			XUI::Tip("Checks everything again and scans the packages on VirusTotal - required: uploading what it does not know and waiting "
				"for the report - then opens the submission, or updates the open one. Nothing is sent without a report of every package.");
		if (running)
		{
			if (XUI::DialogButton("Hide", XUI::Kind::Secondary, true))
				s_Want = false;
			XUI::Tip("The job goes on. Mod > Submit to Mod Browser brings this window back.");
		}
		else if (XUI::DialogButton("Close", XUI::Kind::Secondary))
			s_Want = false;
	}
	DrawWithdrawSheet(view);
	XUI::EndDialog();
}
} // namespace

//------------------------------------------------------------------------------
// public
//------------------------------------------------------------------------------
bool EditorCatalogSubmit::CanRequest()
{
	return EditorProject::Active();
}

void EditorCatalogSubmit::Request()
{
	if (!CanRequest()) return;
	Reap();
	DropStale();
	s_ManifestGen = 0;
	RefreshManifest();
	s_KeyError.clear();
	s_VtError.clear();
	s_KeyTarget.clear();
	RefreshKey();
	s_VtKnown = HasVtKey();
	AccountSource(true);
	s_Want = true;
	// the requirements as they stand now, and where the submission is
	s_StartError.clear();
	if (!Running()) StartFromUI(jbCheck);
}

void EditorCatalogSubmit::DrawUI()
{
	Reap();
	DrawWindow();
}

void EditorCatalogSubmit::Shutdown()
{
	s_Cancel.Cancel();
	if (s_Thread.joinable()) s_Thread.join();
	WipeInputs();
	EditorCatalog::Shutdown();
}

void EditorCatalogSubmit::McpCatalog(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active()) { ErrorJson(out, "no active project"); return; }
	char action[32] = {};
	if (!GrizzlyMCP::GetArg(raw, "action", action, sizeof(action))) strcpy_s(action, "status");
	// request-sized, so no value is cut; wiped before the answer is built
	xr_vector<char> arg(xr_strlen(raw) + 1);
	xr_string err;

	if (0 == xr_strcmp(action, "status")) {}
	else if (0 == xr_strcmp(action, "key_create"))
	{
		xr_string path, password;
		if (GrizzlyMCP::GetArg(raw, "path", arg.data(), (u32)arg.size())) path = arg.data();
		if (GrizzlyMCP::GetArg(raw, "password", arg.data(), (u32)arg.size())) password = arg.data();
		::SecureZeroMemory(arg.data(), arg.size());
		const bool made = EditorCatalog::CreateKey(path.empty() ? EditorCatalog::DefaultKeyPath().c_str() : path.c_str(), password.c_str(), err);
		EditorGithub::WipeString(password);
		if (!made) { ErrorJson(out, err); return; }
	}
	else if (0 == xr_strcmp(action, "key_use"))
	{
		if (!GrizzlyMCP::GetArg(raw, "path", arg.data(), (u32)arg.size()) || !arg[0]) { ErrorJson(out, "key_use needs 'path'"); return; }
		if (!EditorCatalog::UseKeyFile(arg.data(), err)) { ErrorJson(out, err); return; }
	}
	else if (0 == xr_strcmp(action, "key_unlock"))
	{
		const bool given = GrizzlyMCP::GetArg(raw, "password", arg.data(), (u32)arg.size());
		const bool opened = given && EditorCatalog::Unlock(arg.data(), err);
		::SecureZeroMemory(arg.data(), arg.size());
		if (!opened) { ErrorJson(out, given ? err : xr_string("key_unlock needs 'password'")); return; }
	}
	else if (0 == xr_strcmp(action, "vt_key"))
	{
		// the one request that carries a VirusTotal key: it goes into the Credential Manager
		const bool given = GrizzlyMCP::GetArg(raw, "key", arg.data(), (u32)arg.size());
		const bool saved = given && EditorGithub::SaveVtKey(arg.data(), err);
		::SecureZeroMemory(arg.data(), arg.size());
		if (!saved) { ErrorJson(out, given ? err : xr_string("vt_key needs 'key'")); return; }
	}
	else if (0 == xr_strcmp(action, "vt_key_forget"))	EditorGithub::ForgetVtKey();
	else if (0 == xr_strcmp(action, "cancel"))			{ if (Running()) s_Cancel.Cancel(); }
	// the window, for whoever has to look at it from outside: a screenshot, a person. confirm=withdraw
	// puts the withdrawal's question over it - only a click in it withdraws anything
	else if (0 == xr_strcmp(action, "show"))
	{
		if (!s_Want) Request();
		char confirm[16] = {};
		GrizzlyMCP::GetArg(raw, "confirm", confirm, sizeof(confirm));
		if (confirm[0] && 0 != xr_strcmp(confirm, "withdraw")) { ErrorJson(out, "confirm: withdraw expected"); return; }
		s_WantWithdraw = 0 == xr_strcmp(confirm, "withdraw");
	}
	else if (0 == xr_strcmp(action, "hide"))
	{
		s_Want = false;
		s_WantWithdraw = false;
	}
	else
	{
		// withdraw asks nothing here: the caller decided, and the bridge marks the call destructive
		const EJob job = 0 == xr_strcmp(action, "check") ? jbCheck : 0 == xr_strcmp(action, "vt") ? jbVt :
			0 == xr_strcmp(action, "submit") ? jbSubmit : 0 == xr_strcmp(action, "submission") ? jbSubmission :
			0 == xr_strcmp(action, "withdraw") ? jbWithdraw : jbNone;
		if (jbNone == job)
		{
			ErrorJson(out, "unknown action: status, key_create, key_use, key_unlock, vt_key, vt_key_forget, check, vt, submit, submission, "
				"withdraw, cancel, show or hide expected");
			return;
		}
		if (!Start(job, err)) { ErrorJson(out, err); return; }
	}
	// the window shows what MCP changed
	if (s_Want) { RefreshKey(); s_VtKnown = HasVtKey(); }
	StatusJson(out);
}
