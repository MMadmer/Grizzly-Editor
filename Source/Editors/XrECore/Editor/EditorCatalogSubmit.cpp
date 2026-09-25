#include "stdafx.h"
#include "EditorCatalogSubmit.h"
#include "EditorCatalog.h"
#include "EditorGithub.h"
#include "EditorModManifest.h"
#include "EditorProject.h"
#include "EditorVirusTotal.h"
#include "XFinedMCP.h"
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

// what a check learned beyond its findings
struct SChecked
{
	SRelease					rel;
	EditorCatalog::SVtPolicy	policy;		// the index's; the catalog's defaults when it did not answer
	EditorVirusTotal::SScan		vt;
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

enum EJob { jbNone, jbCheck, jbVt, jbSubmit, jbSubmission };
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
	bool						listed;

								SView	() : job(jbNone), state(stIdle), vt_scanned(false), listed(false) {}
};

std::mutex				s_Lock;
SView					s_View;			// guarded by s_Lock
bool					s_Want			= false;	// the window is open
std::thread				s_Thread;
EditorGithub::CCancel	s_Cancel;

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

LPCSTR JobName(EJob job)
{
	switch (job)
	{
	case jbCheck:	return "check";
	case jbVt:		return "vt";
	case jbSubmit:	return "submit";
	case jbSubmission:	return "submission";
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

// A package VirusTotal does not know only waits for Submit's scan - unless the scan was the one
// that could not get it there.
ELevel VtLevel(const xr_string& state, bool scanned)
{
	if (state == "clean")												return lvOk;
	if (state == "warn")												return lvReview;
	if (EditorVirusTotal::Refuses(state) && (scanned || state != "unknown"))	return lvBlock;
	return lvNote;
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
void RunCheck(const SInput& in, bool lookup, SCheck& check, SChecked& seen, EditorGithub::SCatalogIssue& submission,
			  bool& listed)
{
	check = SCheck();
	const EditorMod::SManifest& m = in.manifest;
	SRelease& rel = seen.rel;

	if (!in.key.exists)				Add(check, lvBlock, "key", "No author key: create one above - it signs every release.");
	else if (!in.key.error.empty())	Add(check, lvBlock, "key", "The author key does not read: " + in.key.error + ".");
	else							Add(check, lvOk, "key", "Author key " + in.key.key_id + ".");

	const bool repo_ok = !m.update_github.empty() && EditorMod::ValidateGithubRepo(m.update_github.c_str());
	if (m.update_github.empty())	Add(check, lvBlock, "repository", "No GitHub repository in the manifest: set it in Mod > Edit Manifest.");
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
			Add(check, lvBlock, "account", "The GitHub account does not answer: " + err);
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
				"The latest release could not be read: " + err + ".");
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
		else												Add(check, lvNote, "website", "Website " + website + " could not be reached: " + err + ".");
	}
	if (s_Cancel.Cancelled()) return;

	if (have) CheckLocalRelease(in, rel, check);
	if (s_Cancel.Cancelled()) return;

	// the catalog: is the id free, or already this key's (7.6)
	Stage("Reading the Mod Browser catalog");
	xr_string index, listed_key;
	const bool reached = EditorGithub::FetchCatalogIndex(s_Cancel, index);
	if (reached) EditorCatalog::ReadVtPolicy(index, seen.policy);
	if (!reached)
		Add(check, lvNote, "catalog", "The Mod Browser catalog did not answer: the catalog's own check covers the id.");
	else if (!EditorCatalog::ListedKey(index, m.id, listed_key))
		Add(check, lvOk, "catalog", "The id " + m.id + " is free in the Mod Browser.");
	else if (listed_key == in.key.public_key)
	{
		listed = true;
		Add(check, lvOk, "catalog", "Listed in the Mod Browser with this key.");
	}
	else
		Add(check, lvBlock, "catalog", m.id + " is listed with another author key: sign the release with that key, or submit "
			"the module under another id.");
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
		if (rel.thumbnail.empty())	Add(check, lvNote, "thumbnail", "No thumbnail: the Mod Browser shows the name's first letter (Mod > Edit Manifest).");
		else						Add(check, lvOk, "thumbnail", "Thumbnail.");
		if (rel.tags.empty())		Add(check, lvNote, "tags", "No tags: players who filter by tag will not find it (Mod > Edit Manifest).");
		else						Add(check, lvOk, "tags", "Tags: " + rel.tags + ".");
	}

	// VirusTotal (7.7): the catalog only looks packages up and refuses one VirusTotal does not know,
	// so the scan is the author's, with their own key
	if (have && (lookup || in.vt_key.empty()))
		EditorVirusTotal::Lookup(ScanPackages(in, rel), in.vt_key, seen.policy, s_Cancel, &Stage, seen.vt);
	if (have && in.vt_key.empty())
		Add(check, lvBlock, "virustotal", kNoVtKey);
	else if (have && lookup)
		for (u32 p = 0; p < seen.vt.packages.size(); ++p)
		{
			const EditorVirusTotal::SResult& r = seen.vt.packages[p];
			xr_string text = EditorVirusTotal::Describe(r);
			if (r.state == "unknown")			text += " - not scanned yet: Submit scans it first.";
			else if (r.state == "analysing")	text += ": Submit waits for the report.";
			else								text += ".";
			Add(check, VtLevel(r.state, false), "virustotal", text);
		}
	if (s_Cancel.Cancelled()) return;

	// and where the submission stands
	if (in.signed_in)
	{
		Stage("Reading the submission");
		xr_string err;
		EditorGithub::CatalogIssue(m.id, in.token, s_Cancel, submission, err);
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
	body += "\nChecked by XFined Editor:\n";
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

bool RunSubmit(const SInput& in, SCheck& check, SChecked& seen, EditorGithub::SCatalogIssue& submission, bool& listed,
			   xr_string& err)
{
	RunCheck(in, false, check, seen, submission, listed);
	if (s_Cancel.Cancelled()) { err = "cancelled"; return false; }
	for (u32 i = 0; i < check.findings.size(); ++i)
		if (lvBlock == check.findings[i].level) { err = "not submitted: " + check.findings[i].text; return false; }

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

	const SRelease& rel = seen.rel;
	const xr_string website = (rel.card && !rel.website.empty()) ? rel.website : in.manifest.website;
	EditorGithub::SCatalogRequest rq;
	rq.module_id	= in.manifest.id;
	rq.file_text	= SubmissionText(in, rel, website);
	rq.details		= SubmissionDetails(in, seen, check, website);
	rq.token		= in.token;
	const bool ok = EditorGithub::CatalogSubmit(rq, s_Cancel, [](LPCSTR text) { Stage(text); }, submission, err);
	EditorGithub::WipeString(rq.token);
	return ok;
}

void Worker(EJob job, SInput in)
{
	SCheck check;
	SChecked seen;
	EditorGithub::SCatalogIssue submission;
	bool listed = false, ok = true;
	xr_string err;
	switch (job)
	{
	case jbCheck:	RunCheck(in, true, check, seen, submission, listed); break;
	case jbVt:		RunVt(in, seen.vt, err); ok = err.empty(); break;
	case jbSubmit:	ok = RunSubmit(in, check, seen, submission, listed, err); break;
	case jbSubmission:
	{
		Stage("Reading the submission");
		ok = EditorGithub::CatalogIssue(in.manifest.id, in.token, s_Cancel, submission, err);
		xr_string index, key;
		if (ok && EditorGithub::FetchCatalogIndex(s_Cancel, index))
			listed = EditorCatalog::ListedKey(index, in.manifest.id, key) && key == in.key.public_key;
		break;
	}
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
	// what the catalog and GitHub said about the submission, once a job asked them
	const bool asked = jbSubmission == job ? ok : (jbCheck == job || jbSubmit == job) && check.done;
	if (asked && !cancelled)
	{
		s_View.submission = in.signed_in ? submission : EditorGithub::SCatalogIssue();
		s_View.listed = listed;
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

bool Start(EJob job, xr_string& err)
{
	err.clear();
	Reap();
	if (Running())					{ err = "a Mod Browser job is running already"; return false; }
	if (!EditorProject::Active())	{ err = "no active project"; return false; }
	DropStale();

	SInput in;
	in.root = EditorProject::Root();
	if (!EditorMod::Load(in.root.c_str(), in.manifest))			{ err = "mod.ltx not found in the project"; return false; }
	if (!EditorMod::ValidateId(in.manifest.id.c_str()))			{ err = "the module id is missing or invalid (Mod > Edit Manifest)"; return false; }
	in.key = EditorCatalog::KeyState();
	EditorGithub::SAccount account;
	EditorGithub::ResolveAccount(account);
	NoteAccount(account.source);
	in.signed_in = EditorGithub::asNone != account.source;
	in.token = account.token;
	if ((jbSubmit == job || jbSubmission == job) && !in.signed_in)
		{ err = "not signed in to GitHub: sign in from Mod > Publish Release (MCP: mod_publish action=sign_in or save_token)"; return false; }
	if (jbSubmission != job) EditorGithub::ReadVtKey(in.vt_key);
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
		s_View.error.clear();
	}
	s_Cancel.Reset();
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
	out += NqUtil::Format(",\"submission\":{\"number\":%llu", v.submission.number);
	JsonField(out, "state", v.submission.state.c_str());
	JsonField(out, "verdict", v.submission.verdict.c_str());
	JsonField(out, "verdict_text", v.submission.verdict_text.c_str());
	JsonField(out, "url", v.submission.url.c_str());
	JsonField(out, "note", v.submission.note.c_str());
	out += NqUtil::Format(",\"listed\":%s},\"window\":%s}", JsonBool(v.listed), JsonBool(s_Want));
}

void ErrorJson(xr_string& out, const xr_string& err)
{
	out = "{\"ok\":false,\"error\":\"";
	JsonAppend(out, err.c_str());
	out += "\"}";
}

//------------------------------------------------------------------------------
// the window
//------------------------------------------------------------------------------
const char* kTitle = "XFined Editor - Submit to Mod Browser";

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
bool						s_HideRequested	= false;	// MCP closes the window from outside the ImGui pass

void StartFromUI(EJob job)
{
	xr_string err;
	s_StartError = Start(job, err) ? xr_string() : err;
}

void Hint(LPCSTR text)
{
	if (ImGui::IsItemHovered(ImGuiHoveredFlags_AllowWhenDisabled))
		ImGui::SetTooltip("%s", text);
}

// A path on one line up to `room` wide: folders are dropped from the left until it fits, and
// the whole path is the hint. A path broken in the middle of a name reads as two.
void PathText(const xr_string& path, float room)
{
	xr_string shown = path;
	for (size_t cut = 0; ImGui::CalcTextSize(shown.c_str()).x > room;)
	{
		cut = path.find('\\', cut + 1);
		if (cut == xr_string::npos) break;
		shown = "..." + path.substr(cut);
	}
	ImGui::TextDisabled("%s", shown.c_str());
	Hint(path.c_str());
}

// what is left of the window's text width after the cursor and `reserve`
float RoomLeft(float reserve)
{
	return ImGui::GetFontSize() * 40.f - ImGui::GetCursorPosX() - reserve;
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
		const COMDLG_FILTERSPEC types[] = { { L"XFined author key (*.xkey)", L"*.xkey" }, { L"All files", L"*.*" } };
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

ImU32 LevelColor(ELevel level)
{
	switch (level)
	{
	case lvOk:		return IM_COL32(110, 200, 110, 255);
	case lvReview:	return IM_COL32(230, 190, 80, 255);
	case lvBlock:	return IM_COL32(235, 95, 90, 255);
	}
	return IM_COL32(150, 150, 150, 255);
}

// a status dot in front of a wrapped line
void StatusLine(ELevel level, LPCSTR text)
{
	const float h = ImGui::GetTextLineHeight();
	const ImVec2 at = ImGui::GetCursorScreenPos();
	ImGui::GetWindowDrawList()->AddCircleFilled(ImVec2(at.x + h * 0.35f, at.y + h * 0.5f), h * 0.22f, LevelColor(level));
	ImGui::Dummy(ImVec2(h * 0.9f, h));
	ImGui::SameLine(0.f, 0.f);
	ImGui::TextWrapped("%s", text);
}

void DrawKey()
{
	ImGui::SeparatorText("Author key");
	const ImVec4 bad(1.f, 0.4f, 0.4f, 1.f), warn(1.f, 0.8f, 0.3f, 1.f);
	if (!s_Key.exists)
	{
		ImGui::TextWrapped("No author key yet. The key signs every release of the module, and the Mod Browser lists "
			"signed releases only. It is a file you keep - never part of the project.");
		PathText(s_KeyTarget, RoomLeft(ImGui::CalcTextSize("Change...").x + ImGui::GetFontSize() * 2.f));
		ImGui::SameLine();
		ImGui::BeginDisabled(s_PickQueued);
		if (ImGui::SmallButton("Change...")) s_PickQueued = UI->DeferUIWork(&PickKeyTarget);
		ImGui::EndDisabled();
		const float field = ImGui::GetFontSize() * 12.f;
		ImGui::SetNextItemWidth(field);
		ImGui::InputTextWithHint("##password", "password (optional)", s_Password, sizeof(s_Password), ImGuiInputTextFlags_Password);
		Hint("Protects the file if it ever leaves your hands. The editor asks for it once per session.\n"
			 "Forgotten, it cannot be recovered - and neither can the key.");
		ImGui::SameLine();
		ImGui::SetNextItemWidth(field);
		ImGui::InputTextWithHint("##repeat", "repeat the password", s_Repeat, sizeof(s_Repeat), ImGuiInputTextFlags_Password);
		const bool differ = 0 != strcmp(s_Password, s_Repeat);
		ImGui::BeginDisabled(differ);
		if (ImGui::Button("Create Key"))
		{
			if (EditorCatalog::CreateKey(s_KeyTarget.c_str(), s_Password, s_KeyError))
			{
				s_KeyError.clear();
				s_KeyCreated = true;
			}
			WipeInputs();
			RefreshKey();
		}
		ImGui::EndDisabled();
		ImGui::SameLine();
		ImGui::BeginDisabled(s_PickQueued);
		if (ImGui::Button("Use Key File...")) s_PickQueued = UI->DeferUIWork(&PickKeyFile);
		ImGui::EndDisabled();
		Hint("The key file from another computer or its copy.");
		if (differ && s_Repeat[0]) ImGui::TextColored(bad, "The passwords differ.");
	}
	else if (!s_Key.error.empty())
	{
		ImGui::TextColored(bad, "The key file does not read: %s", s_Key.error.c_str());
		PathText(s_Key.path, RoomLeft(0.f));
		ImGui::BeginDisabled(s_PickQueued);
		if (ImGui::Button("Use Another Key File...")) s_PickQueued = UI->DeferUIWork(&PickKeyFile);
		ImGui::EndDisabled();
	}
	else
	{
		ImGui::Text("Key %s", s_Key.key_id.c_str());
		ImGui::SameLine();
		PathText(s_Key.path, RoomLeft(ImGui::CalcTextSize("Show in Folder").x + ImGui::GetFontSize() * 2.f));
		ImGui::SameLine();
		if (ImGui::SmallButton("Show in Folder"))
			::ShellExecuteA(0, "open", "explorer.exe", ("/select,\"" + s_Key.path + "\"").c_str(), 0, SW_SHOWNORMAL);
		if (s_Key.locked)
		{
			ImGui::SetNextItemWidth(ImGui::GetFontSize() * 12.f);
			const bool enter = ImGui::InputTextWithHint("##unlock", "key password", s_Password, sizeof(s_Password),
				ImGuiInputTextFlags_Password | ImGuiInputTextFlags_EnterReturnsTrue);
			ImGui::SameLine();
			if (ImGui::Button("Unlock") || enter)
			{
				if (EditorCatalog::Unlock(s_Password, s_KeyError)) s_KeyError.clear();
				WipeInputs();
				RefreshKey();
			}
		}
		ImGui::PushStyleColor(ImGuiCol_Text, s_KeyCreated ? warn : ImGui::GetStyleColorVec4(ImGuiCol_TextDisabled));
		ImGui::TextWrapped("Keep a copy of this file somewhere safe - a USB stick, a password manager. Without it no "
			"update of this module can be signed.");
		ImGui::PopStyleColor();
		ImGui::BeginDisabled(s_PickQueued);
		if (ImGui::SmallButton("Use Another Key File...")) s_PickQueued = UI->DeferUIWork(&PickKeyFile);
		ImGui::EndDisabled();
	}
	if (!s_KeyError.empty()) ImGui::TextColored(bad, "%s", s_KeyError.c_str());
}

void DrawRequirements(const SView& view)
{
	ImGui::SeparatorText("Requirements");
	const bool checking = stRunning == view.state && (jbCheck == view.job || jbSubmit == view.job);
	if (checking)
		ImGui::TextDisabled("Checking... %s", view.stage.c_str());
	else if (!view.check.done)
		ImGui::TextDisabled("Not checked yet.");
	for (u32 i = 0; i < view.check.findings.size(); ++i)
		StatusLine(view.check.findings[i].level, view.check.findings[i].text.c_str());
	if (!s_StartError.empty())
		ImGui::TextColored(ImVec4(1.f, 0.4f, 0.4f, 1.f), "%s", s_StartError.c_str());
	ImGui::BeginDisabled(stRunning == view.state);
	if (ImGui::Button("Check Again")) StartFromUI(jbCheck);
	ImGui::EndDisabled();
	Hint("Reads the latest release the way the game does and checks what the Mod Browser requires.");
}

void DrawVirusTotal(const SView& view)
{
	ImGui::SeparatorText("VirusTotal");
	if (!s_VtKnown)
	{
		ImGui::TextWrapped("Required: the catalog lists a package only when VirusTotal knows it, and every release is scanned "
			"with your own VirusTotal key. A free account at virustotal.com has one.");
		ImGui::SetNextItemWidth(ImGui::GetFontSize() * 20.f);
		ImGui::InputTextWithHint("##vt_key", "VirusTotal API key", s_VtInput, sizeof(s_VtInput), ImGuiInputTextFlags_Password);
		Hint("Kept in the Windows Credential Manager - never in the project.");
		ImGui::SameLine();
		ImGui::BeginDisabled(!s_VtInput[0]);
		if (ImGui::Button("Save Key"))
		{
			if (EditorGithub::SaveVtKey(s_VtInput, s_VtError)) s_VtError.clear();
			WipeInputs();
			s_VtKnown = HasVtKey();
		}
		ImGui::EndDisabled();
		if (!s_VtError.empty()) ImGui::TextColored(ImVec4(1.f, 0.4f, 0.4f, 1.f), "%s", s_VtError.c_str());
		return;
	}
	if (stRunning == view.state && (jbVt == view.job || jbSubmit == view.job))
		ImGui::TextDisabled("Scanning... %s", view.stage.c_str());
	else if (view.virustotal.packages.empty())
		ImGui::TextDisabled("Not scanned yet.");
	for (u32 i = 0; i < view.virustotal.packages.size(); ++i)
	{
		const EditorVirusTotal::SResult& r = view.virustotal.packages[i];
		ImGui::PushID((int)i);
		StatusLine(VtLevel(r.state, view.vt_scanned), (EditorVirusTotal::Describe(r) + ".").c_str());
		ImGui::SameLine();
		if (ImGui::SmallButton("Report"))
			::ShellExecuteA(0, "open", EditorGithub::VtReportPage(r.package.sha256).c_str(), 0, 0, SW_SHOWNORMAL);
		ImGui::PopID();
	}
	ImGui::BeginDisabled(stRunning == view.state);
	if (ImGui::Button("Scan"))
	{
		xr_string err;
		s_VtError = Start(jbVt, err) ? xr_string() : err;
	}
	ImGui::EndDisabled();
	Hint("Looks every package of the latest release up on VirusTotal, uploads one it does not know and waits for its\n"
		 "report - a few minutes. VirusTotal shares uploaded files with the security industry.");
	ImGui::SameLine();
	if (ImGui::SmallButton("Forget Key"))
	{
		EditorGithub::ForgetVtKey();
		s_VtKnown = HasVtKey();
	}
	Hint("Removes the key the editor saved. A key from VT_API_KEY stays in effect.");
	if (!s_VtError.empty()) ImGui::TextColored(ImVec4(1.f, 0.4f, 0.4f, 1.f), "%s", s_VtError.c_str());
}

void DrawSubmission(const SView& view)
{
	ImGui::SeparatorText("Submission");
	const EditorGithub::SCatalogIssue& submission = view.submission;
	if (!submission.number)
		ImGui::TextDisabled("Not submitted yet.");
	else
	{
		// Open, the catalog's newest verdict says what it waits for. Closed, only an accepted or a
		// refused one says what became of it: the author may have closed it after a waiting one.
		xr_string text, verdict;
		ELevel level = lvNote, verdict_level = lvNote;
		if (submission.state == "open")
		{
			text = NqUtil::Format("#%llu is open: the catalog checks it.", submission.number);
			verdict = submission.verdict_text;
			verdict_level = submission.verdict == "accepted" ? lvOk : submission.verdict == "refused" ? lvBlock : lvReview;
		}
		else if (submission.verdict == "accepted")
		{
			level = lvOk;
			text = view.listed ? NqUtil::Format("#%llu was accepted: the module is in the Mod Browser.", submission.number) :
				NqUtil::Format("#%llu was accepted: the module appears in the Mod Browser within minutes.", submission.number);
		}
		else if (submission.verdict == "refused")
		{
			level = verdict_level = lvBlock;
			text = NqUtil::Format("#%llu was refused.", submission.number);
			verdict = submission.verdict_text;
		}
		else
			text = NqUtil::Format("#%llu was closed.", submission.number);
		StatusLine(level, text.c_str());
		if (!verdict.empty()) StatusLine(verdict_level, verdict.c_str());
		if (!submission.note.empty()) StatusLine(lvNote, submission.note.c_str());
		if (ImGui::Button("Open Submission")) ::ShellExecuteA(0, "open", submission.url.c_str(), 0, 0, SW_SHOWNORMAL);
		ImGui::SameLine();
	}
	ImGui::BeginDisabled(stRunning == view.state || EditorGithub::asNone == s_AccountSource);
	if (ImGui::Button("Refresh")) StartFromUI(jbSubmission);
	ImGui::EndDisabled();
	if (stFailed == view.state && !view.error.empty())
		ImGui::TextColored(ImVec4(1.f, 0.4f, 0.4f, 1.f), "%s", view.error.c_str());
}

void DrawWindow()
{
	static bool was_open = false;
	if (!s_Want)
	{
		if (was_open) WipeInputs();
		was_open = false;
		return;
	}
	was_open = true;
	const float em = ImGui::GetFontSize();
	ImGui::SetNextWindowSize(ImVec2(em * 42.f, 0), ImGuiCond_Appearing);
	if (!ImGui::BeginPopupModal(kTitle, &s_Want, ImGuiWindowFlags_AlwaysAutoResize, true))
		return;
	if (s_HideRequested)
	{
		s_HideRequested = false;
		s_Want = false;
		ImGui::CloseCurrentPopup();
		ImGui::EndPopup();
		return;
	}
	RefreshManifest();
	const SView view = Snapshot();
	ImGui::PushTextWrapPos(em * 40.f);

	ImGui::TextUnformatted(s_Manifest.name.empty() ? s_Manifest.id.c_str() : s_Manifest.name.c_str());
	ImGui::SameLine();
	ImGui::TextDisabled("(%s)", s_Manifest.id.c_str());
	ImGui::TextDisabled("Lists the module in the Mod Browser of Dead Air: Refined. Submissions that pass the checks of %s "
		"are listed automatically.", EditorGithub::CatalogRepo().c_str());

	DrawKey();
	DrawRequirements(view);
	DrawVirusTotal(view);
	DrawSubmission(view);

	ImGui::Separator();
	const bool signed_in = EditorGithub::asNone != s_AccountSource;
	const bool ready = view.check.done && !view.check.blocking && signed_in && stRunning != view.state;
	ImGui::BeginDisabled(!ready);
	if (ImGui::Button(view.submission.number && view.submission.state == "open" ? "Update Submission" : "Submit", ImVec2(em * 10.f, 0)))
		StartFromUI(jbSubmit);
	ImGui::EndDisabled();
	Hint(ready ? "Checks everything again and scans the packages on VirusTotal - uploading what it does not know and\n"
				 "waiting for the report - then opens the submission, or updates the open one." :
		"Available once every requirement above is met.");
	ImGui::SameLine();
	if (stRunning == view.state && jbSubmit == view.job)
	{
		if (ImGui::Button("Cancel", ImVec2(em * 6.f, 0))) s_Cancel.Cancel();
		ImGui::SameLine();
		ImGui::TextDisabled("%s", view.stage.c_str());
	}
	else if (ImGui::Button("Close", ImVec2(em * 6.f, 0)))
	{
		s_Want = false;
		ImGui::CloseCurrentPopup();
	}
	if (signed_in)
		ImGui::TextDisabled("Submitting opens a public issue at github.com/%s%s%s.", EditorGithub::CatalogRepo().c_str(),
			view.login.empty() ? "" : " from ", view.login.c_str());

	ImGui::PopTextWrapPos();
	ImGui::EndPopup();
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
	if (!XFinedMCP::GetArg(raw, "action", action, sizeof(action))) strcpy_s(action, "status");
	// request-sized, so no value is cut; wiped before the answer is built
	xr_vector<char> arg(xr_strlen(raw) + 1);
	xr_string err;

	if (0 == xr_strcmp(action, "status")) {}
	else if (0 == xr_strcmp(action, "key_create"))
	{
		xr_string path, password;
		if (XFinedMCP::GetArg(raw, "path", arg.data(), (u32)arg.size())) path = arg.data();
		if (XFinedMCP::GetArg(raw, "password", arg.data(), (u32)arg.size())) password = arg.data();
		::SecureZeroMemory(arg.data(), arg.size());
		const bool made = EditorCatalog::CreateKey(path.empty() ? EditorCatalog::DefaultKeyPath().c_str() : path.c_str(), password.c_str(), err);
		EditorGithub::WipeString(password);
		if (!made) { ErrorJson(out, err); return; }
	}
	else if (0 == xr_strcmp(action, "key_use"))
	{
		if (!XFinedMCP::GetArg(raw, "path", arg.data(), (u32)arg.size()) || !arg[0]) { ErrorJson(out, "key_use needs 'path'"); return; }
		if (!EditorCatalog::UseKeyFile(arg.data(), err)) { ErrorJson(out, err); return; }
	}
	else if (0 == xr_strcmp(action, "key_unlock"))
	{
		const bool given = XFinedMCP::GetArg(raw, "password", arg.data(), (u32)arg.size());
		const bool opened = given && EditorCatalog::Unlock(arg.data(), err);
		::SecureZeroMemory(arg.data(), arg.size());
		if (!opened) { ErrorJson(out, given ? err : xr_string("key_unlock needs 'password'")); return; }
	}
	else if (0 == xr_strcmp(action, "vt_key"))
	{
		// the one request that carries a VirusTotal key: it goes into the Credential Manager
		const bool given = XFinedMCP::GetArg(raw, "key", arg.data(), (u32)arg.size());
		const bool saved = given && EditorGithub::SaveVtKey(arg.data(), err);
		::SecureZeroMemory(arg.data(), arg.size());
		if (!saved) { ErrorJson(out, given ? err : xr_string("vt_key needs 'key'")); return; }
	}
	else if (0 == xr_strcmp(action, "vt_key_forget"))	EditorGithub::ForgetVtKey();
	else if (0 == xr_strcmp(action, "cancel"))			{ if (Running()) s_Cancel.Cancel(); }
	// the window, for whoever has to look at it from outside: a screenshot, a person
	else if (0 == xr_strcmp(action, "show"))			{ s_HideRequested = false; if (!s_Want) Request(); }
	else if (0 == xr_strcmp(action, "hide"))			s_HideRequested = s_Want;
	else
	{
		const EJob job = 0 == xr_strcmp(action, "check") ? jbCheck : 0 == xr_strcmp(action, "vt") ? jbVt :
			0 == xr_strcmp(action, "submit") ? jbSubmit : 0 == xr_strcmp(action, "submission") ? jbSubmission : jbNone;
		if (jbNone == job)
		{
			ErrorJson(out, "unknown action: status, key_create, key_use, key_unlock, vt_key, vt_key_forget, check, vt, submit, submission, "
				"cancel, show or hide expected");
			return;
		}
		if (!Start(job, err)) { ErrorJson(out, err); return; }
	}
	// the window shows what MCP changed
	if (s_Want) { RefreshKey(); s_VtKnown = HasVtKey(); }
	StatusJson(out);
}
