#pragma once

// Mod > Publish Release: packages the project and publishes the result as a
// GitHub release of the manifest's [update] github repository - what the game's
// Mods menu updates from. The dialog, the upload job and the MCP command live
// here; the GitHub side is EditorGithub.
//
// The repository is bound once, in mod.ltx. The account is bound once per
// Windows user: a token in the Credential Manager, the GitHub CLI or an
// environment variable. A token is never written into the project, a log or an
// MCP answer, and no MCP request carries one.

namespace EditorPublish
{
	// opens the dialog - or, while a job runs, its progress
	ECORE_API void	RequestPublish	();
	ECORE_API bool	CanPublish		();
	// toolbar label: "Publish Release", or the progress of the running job
	ECORE_API void	ButtonText		(char* dst, u32 size);
	ECORE_API void	DrawUI			();
	// the editor is closing: cancels the job and the sign-in and waits for both
	ECORE_API void	Shutdown		();

	// {"action":"start"|"status"|"cancel"|"account", ...}; start answers as soon
	// as the release is packaged and the upload runs, status reports the rest
	ECORE_API void	McpPublish		(LPCSTR raw, xr_string& out);
}
