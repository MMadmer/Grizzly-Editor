#pragma once
#include "EditorCatalog.h"
#include "EditorGithub.h"

// The author's VirusTotal scan of a release (game repo docs/dead-air/MOD_CATALOG.md 11). The
// catalog only looks packages up by their SHA-256: it refuses a submission whose package
// VirusTotal does not know and holds back such a release of a listed module. Getting the packages
// known is the author's job, with their own free key - Submit to Mod Browser scans before it sends
// anything, Publish Release before a release of a module the Mod Browser lists goes out. Both run
// this one routine on their workers; the requests keep to the free API's pace (EditorGithub).
namespace EditorVirusTotal
{
	struct SPackage
	{
		xr_string	name;
		xr_string	path;		// this computer's copy, uploaded only when its hash is `sha256`
		u64			size;
		xr_string	sha256;		// what the descriptor pins, lowercase hex

					SPackage	() : size(0) {}
	};

	// state: clean, warn, block	the report, judged by the catalog's policy
	//        analysing			VirusTotal has the file, its report is not out yet
	//        unknown				VirusTotal does not know it (a scan: nor could it be uploaded)
	//        too_large			larger than VirusTotal takes
	//        unavailable			VirusTotal did not answer: the network, the rate, the key refused
	//        no_key				there is no VirusTotal key
	struct SResult
	{
		SPackage					package;
		xr_string					state;
		EditorCatalog::SVtVerdict	verdict;
		xr_string					error;
	};

	struct SScan
	{
		// none (nothing asked), scanning, cancelled, or the state of the package that decides
		xr_string			state;
		xr_vector<SResult>	packages;

							SScan	() : state("none") {}
	};

	typedef std::function<void(LPCSTR stage)> TStage;

	// what VirusTotal knows now; nothing is uploaded, nothing waited for
	void		Lookup		(const xr_vector<SPackage>& packages, const xr_string& key,
							 const EditorCatalog::SVtPolicy& policy, EditorGithub::CCancel& cancel,
							 const TStage& stage, SScan& out);
	// every package looked up, the ones VirusTotal does not know uploaded from this computer, and
	// their reports waited for - ten minutes at most
	void		Scan		(const xr_vector<SPackage>& packages, const xr_string& key,
							 const EditorCatalog::SVtPolicy& policy, EditorGithub::CCancel& cancel,
							 const TStage& stage, SScan& out);

	// the catalog would refuse the release or hold it back: nothing may go out
	bool		Refuses		(const xr_string& state);
	// the package whose state is the scan's; 0 when none is
	const SResult* Deciding	(const SScan& scan);
	// one sentence about a package, without the final period
	xr_string	Describe	(const SResult& r);
	// appends {"state":...,"packages":[{"name","sha256","size","state","engines",...}]}
	void		Json		(xr_string& out, const SScan& scan);

	// a local file's SHA-256, lowercase hex; false when it does not read or the job is cancelled
	bool		FileSha256	(const xr_string& path, EditorGithub::CCancel& cancel, xr_string& hex, u64& size);
}
