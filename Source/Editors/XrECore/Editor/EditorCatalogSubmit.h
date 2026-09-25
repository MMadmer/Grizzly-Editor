#pragma once

// Mod > Submit to Mod Browser: lists the project's module in the Mod Browser of Dead Air:
// Refined (game repo docs/dead-air/MOD_CATALOG.md, sections 7 and 10). One job at a time runs
// on a worker, whether the window or MCP started it: the listing requirements checked against
// the latest GitHub release, the VirusTotal scan every submission needs (EditorVirusTotal, with the
// author's own key: the catalog only looks packages up), the submission - an issue of the catalog -
// and following that issue and the verdict the catalog comments on it.
//
// The author key and the card are EditorCatalog, the network is EditorGithub. Like every MCP
// path, mod_catalog asks nothing and opens no window; its secrets (a key password, a VirusTotal
// key) go in and never come out.

namespace EditorCatalogSubmit
{
	ECORE_API bool	CanRequest	();
	// opens the window and checks the requirements at once
	ECORE_API void	Request		();
	ECORE_API void	DrawUI		();
	// the editor is closing: the job is cancelled and waited for, the open author key wiped
	ECORE_API void	Shutdown	();
	// {"action": "status" | "key_create" | "key_use" | "key_unlock" | "vt_key" | "vt_key_forget" |
	//  "check" | "vt" | "submit" | "submission" | "cancel" | "show" | "hide", "path", "password", "key"};
	// the jobs answer at once with job.pending = true, status reports the rest; show and hide
	// open and close the window for whoever looks at it from outside
	ECORE_API void	McpCatalog	(LPCSTR raw, xr_string& out);
}
