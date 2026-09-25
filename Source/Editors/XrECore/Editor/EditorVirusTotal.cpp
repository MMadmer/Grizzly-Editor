#include "stdafx.h"
#include "EditorVirusTotal.h"
#include "Nq/NqUtil.h"
#include <bcrypt.h>

namespace
{
using EditorVirusTotal::SResult;
using EditorVirusTotal::SScan;
using EditorVirusTotal::TStage;

// the most decisive state first: a scan takes the first one any of its packages has
const char* const kOrder[] = { "no_key", "block", "too_large", "unknown", "unavailable", "analysing", "warn", "clean" };

// a QA run (-qa_github) shortens the waits, so the paths are tested in seconds
u32 QaMs(LPCSTR variable, u32 fallback)
{
	char text[32] = {};
	if (EditorGithub::QaRun() && ::GetEnvironmentVariableA(variable, text, sizeof(text)) && text[0])
		return (u32)strtoul(text, 0, 10);
	return fallback;
}

void Pause(EditorGithub::CCancel& cancel, u32 ms)
{
	for (u32 waited = 0; waited < ms && !cancel.Cancelled(); waited += 100)
		::Sleep(ms - waited < 100 ? ms - waited : 100);
}

void Say(const TStage& stage, const xr_string& text)
{
	if (stage) stage(text.c_str());
}

// A refusal for the rate is waited out a few times - the key may be busy elsewhere, the game's
// Mods window takes one too; after that the quota counts as used up.
template <class TCall>
bool WithinRate(TCall call, u32& status, EditorGithub::CCancel& cancel, const TStage& stage)
{
	const u32 pause = QaMs("XFINED_VT_RATE_WAIT_MS", 20000);
	for (u32 attempt = 0;; ++attempt)
	{
		const bool ok = call();
		if (ok || 429 != status || 3 == attempt || cancel.Cancelled()) return ok;
		Say(stage, "Waiting for VirusTotal's rate limit");
		Pause(cancel, pause);
	}
}

// the key refused or its quota used up: asking about another package would fail the same way
bool KeyProblem(u32 status)
{
	return 401 == status || 403 == status || 429 == status;
}

void Begin(const xr_vector<EditorVirusTotal::SPackage>& packages, SScan& out)
{
	out = SScan();
	for (u32 i = 0; i < packages.size(); ++i)
	{
		SResult r;
		r.package = packages[i];
		out.packages.push_back(r);
	}
}

void Fail(SScan& out, u32 i, u32 status, const xr_string& err)
{
	for (u32 p = 0; p < out.packages.size(); ++p)
		if (p == i || (KeyProblem(status) && out.packages[p].state.empty()))
		{
			out.packages[p].state = "unavailable";
			out.packages[p].error = err;
		}
}

void Judge(SResult& r, const EditorGithub::SVtReport& report, const EditorCatalog::SVtPolicy& policy)
{
	xr_vector<EditorCatalog::SVtEngine> results;
	for (u32 i = 0; i < report.results.size(); ++i)
	{
		EditorCatalog::SVtEngine engine;
		engine.engine	= report.results[i].engine;
		engine.category	= report.results[i].category;
		results.push_back(engine);
	}
	r.verdict	= EditorCatalog::VtVerdict(results, policy);
	r.state		= r.verdict.verdict;
	r.error.clear();
}

void Settle(SScan& out, bool cancelled)
{
	if (cancelled) { out.state = "cancelled"; return; }
	u32 first = _countof(kOrder);
	for (u32 p = 0; p < out.packages.size(); ++p)
		for (u32 k = 0; k < first; ++k)
			if (out.packages[p].state == kOrder[k]) { first = k; break; }
	out.state = first < _countof(kOrder) ? kOrder[first] : "none";
}

bool Look(const SResult& r, const xr_string& key, EditorGithub::CCancel& cancel, const TStage& stage,
		  EditorGithub::SVtReport& report, xr_string& err)
{
	return WithinRate([&]() { return EditorGithub::VtLookup(r.package.sha256, key, cancel, report, err); },
		report.status, cancel, stage);
}
} // namespace

void EditorVirusTotal::Lookup(const xr_vector<SPackage>& packages, const xr_string& key,
							  const EditorCatalog::SVtPolicy& policy, EditorGithub::CCancel& cancel,
							  const TStage& stage, SScan& out)
{
	Begin(packages, out);
	for (u32 i = 0; i < out.packages.size() && !cancel.Cancelled(); ++i)
	{
		SResult& r = out.packages[i];
		if (key.empty()) { r.state = "no_key"; continue; }
		if (!r.state.empty()) continue;
		Say(stage, "Looking up " + r.package.name + " on VirusTotal");
		EditorGithub::SVtReport report;
		xr_string err;
		if (!Look(r, key, cancel, stage, report, err))	Fail(out, i, report.status, err);
		else if (report.known && !report.queued)		Judge(r, report, policy);
		else if (report.known)							r.state = "analysing";
		else											r.state = EditorGithub::VtTooLarge(r.package.size) ? "too_large" : "unknown";
	}
	Settle(out, cancel.Cancelled());
}

void EditorVirusTotal::Scan(const xr_vector<SPackage>& packages, const xr_string& key,
							const EditorCatalog::SVtPolicy& policy, EditorGithub::CCancel& cancel,
							const TStage& stage, SScan& out)
{
	Begin(packages, out);
	if (key.empty())
	{
		for (u32 i = 0; i < out.packages.size(); ++i) out.packages[i].state = "no_key";
		Settle(out, false);
		return;
	}

	// 1. what VirusTotal knows; what it does not know goes up from this computer
	xr_vector<u32> waiting;
	for (u32 i = 0; i < out.packages.size() && !cancel.Cancelled(); ++i)
	{
		SResult& r = out.packages[i];
		if (!r.state.empty()) continue;
		Say(stage, "Looking up " + r.package.name + " on VirusTotal");
		EditorGithub::SVtReport report;
		xr_string err;
		if (!Look(r, key, cancel, stage, report, err))	{ Fail(out, i, report.status, err); continue; }
		if (report.known && !report.queued)				{ Judge(r, report, policy); continue; }
		if (!report.known)
		{
			if (EditorGithub::VtTooLarge(r.package.size)) { r.state = "too_large"; continue; }
			// only the very bytes the descriptor pins go up
			Say(stage, "Hashing " + r.package.name);
			xr_string sha;
			u64 size = 0;
			if (r.package.path.empty() || !FileSha256(r.package.path, cancel, sha, size) || sha != r.package.sha256)
			{
				if (cancel.Cancelled()) break;
				r.state = "unknown";
				r.error = "this computer has no copy of it to upload";
				continue;
			}
			Say(stage, "Uploading " + r.package.name + " to VirusTotal");
			u32 status = 0;
			if (!WithinRate([&]() { return EditorGithub::VtUpload(r.package.path, key, cancel, status, err); }, status, cancel, stage))
			{
				Fail(out, i, status, err);
				continue;
			}
		}
		waiting.push_back(i);
	}

	// 2. The reports, usually out within minutes. A file VirusTotal has never seen answers "not
	// found" until its first analysis ends, so that is waited out like a report without results.
	const ULONGLONG start = ::GetTickCount64();
	const ULONGLONG wait = QaMs("XFINED_VT_SCAN_WAIT_MS", 10 * 60 * 1000);
	while (!waiting.empty() && !cancel.Cancelled() && ::GetTickCount64() - start < wait)
	{
		for (u32 w = 0; w < waiting.size() && !cancel.Cancelled();)
		{
			SResult& r = out.packages[waiting[w]];
			if (r.state.empty())
			{
				const u32 seconds = u32((::GetTickCount64() - start) / 1000);
				Say(stage, NqUtil::Format("Waiting for VirusTotal's report on %s (%u:%02u)", r.package.name.c_str(),
					seconds / 60, seconds % 60));
				EditorGithub::SVtReport report;
				xr_string err;
				if (Look(r, key, cancel, stage, report, err))
				{
					if (report.known && !report.queued) Judge(r, report, policy);
					else r.error.clear();
				}
				// a lost answer is asked again next round; a refused key or a used-up quota ends the wait
				else if (KeyProblem(report.status))	Fail(out, waiting[w], report.status, err);
				else								r.error = err;
			}
			if (r.state.empty())	++w;
			else					waiting.erase(waiting.begin() + w);
		}
	}
	// no report in time: VirusTotal is still at it, or stopped answering
	for (u32 w = 0; w < waiting.size(); ++w)
	{
		SResult& r = out.packages[waiting[w]];
		if (r.state.empty()) r.state = r.error.empty() ? "analysing" : "unavailable";
	}
	Settle(out, cancel.Cancelled());
}

bool EditorVirusTotal::Refuses(const xr_string& state)
{
	return state == "no_key" || state == "block" || state == "too_large" || state == "unknown";
}

const EditorVirusTotal::SResult* EditorVirusTotal::Deciding(const SScan& scan)
{
	for (u32 i = 0; i < scan.packages.size(); ++i)
		if (scan.packages[i].state == scan.state) return &scan.packages[i];
	return 0;
}

xr_string EditorVirusTotal::Describe(const SResult& r)
{
	const LPCSTR name = r.package.name.c_str();
	xr_string flagged;
	for (u32 f = 0; f < r.verdict.flagged.size(); ++f)
		flagged += (f ? ", " : "") + r.verdict.flagged[f];
	if (r.state == "clean")			return NqUtil::Format("VirusTotal: %s is clean (%u engines)", name, r.verdict.engines);
	if (r.state == "warn")			return NqUtil::Format("VirusTotal warns about %s: flagged by %s", name, flagged.c_str());
	if (r.state == "block")			return NqUtil::Format("VirusTotal blocks %s: flagged by %s", name, flagged.c_str());
	if (r.state == "analysing")		return NqUtil::Format("VirusTotal is still analysing %s", name);
	if (r.state == "too_large")		return NqUtil::Format("%s is larger than the 650 MB VirusTotal takes", name);
	if (r.state == "unavailable")	return NqUtil::Format("VirusTotal did not answer about %s: %s", name, r.error.c_str());
	if (r.state == "no_key")		return NqUtil::Format("No VirusTotal key to scan %s", name);
	if (r.state == "unknown")
		return r.error.empty() ? NqUtil::Format("VirusTotal does not know %s", name) :
			NqUtil::Format("VirusTotal does not know %s, and %s", name, r.error.c_str());
	return NqUtil::Format("%s was not scanned", name);
}

void EditorVirusTotal::Json(xr_string& out, const SScan& scan)
{
	out += "{\"state\":";
	NqUtil::JsonString(out, scan.state);
	out += ",\"packages\":[";
	for (u32 i = 0; i < scan.packages.size(); ++i)
	{
		const SResult& r = scan.packages[i];
		out += i ? ",{\"name\":" : "{\"name\":";
		NqUtil::JsonString(out, r.package.name);
		out += ",\"sha256\":";
		NqUtil::JsonString(out, r.package.sha256);
		out += NqUtil::Format(",\"size\":%llu,\"state\":", r.package.size);
		NqUtil::JsonString(out, r.state);
		out += NqUtil::Format(",\"engines\":%u,\"trusted_malicious\":%u,\"trusted_suspicious\":%u,\"other_malicious\":%u,\"flagged\":",
			r.verdict.engines, r.verdict.trusted_malicious, r.verdict.trusted_suspicious, r.verdict.other_malicious);
		NqUtil::JsonStringArray(out, r.verdict.flagged);
		out += ",\"url\":";
		NqUtil::JsonString(out, EditorGithub::VtReportPage(r.package.sha256));
		out += ",\"error\":";
		NqUtil::JsonString(out, r.error);
		out += "}";
	}
	out += "]}";
}

bool EditorVirusTotal::FileSha256(const xr_string& path, EditorGithub::CCancel& cancel, xr_string& hex, u64& size)
{
	hex.clear();
	size = 0;
	HANDLE f = ::CreateFileA(path.c_str(), GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, 0);
	if (INVALID_HANDLE_VALUE == f) return false;
	BCRYPT_ALG_HANDLE alg = 0;
	BCRYPT_HASH_HANDLE hash = 0;
	bool ok = BCRYPT_SUCCESS(::BCryptOpenAlgorithmProvider(&alg, BCRYPT_SHA256_ALGORITHM, 0, 0)) &&
		BCRYPT_SUCCESS(::BCryptCreateHash(alg, &hash, 0, 0, 0, 0, 0));
	xr_vector<u8> buf(1u << 20);
	while (ok && !cancel.Cancelled())
	{
		DWORD got = 0;
		if (!::ReadFile(f, buf.data(), (DWORD)buf.size(), &got, 0)) { ok = false; break; }
		if (!got) break;
		ok = BCRYPT_SUCCESS(::BCryptHashData(hash, buf.data(), got, 0));
		size += got;
	}
	u8 digest[32];
	ok = ok && !cancel.Cancelled() && BCRYPT_SUCCESS(::BCryptFinishHash(hash, digest, 32, 0));
	if (hash) ::BCryptDestroyHash(hash);
	if (alg) ::BCryptCloseAlgorithmProvider(alg, 0);
	::CloseHandle(f);
	if (ok)
		for (u32 i = 0; i < 32; ++i) hex += NqUtil::Format("%02x", (u32)digest[i]);
	return ok;
}
