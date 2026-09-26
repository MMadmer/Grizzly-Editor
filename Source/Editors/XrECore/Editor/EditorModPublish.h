#pragma once

// Mod > Publish Release: packages the project and publishes the result as a
// GitHub release of the manifest's [update] github repository - what the game's
// Mods menu updates from. The dialog, the upload job, the Releases tab and the
// MCP commands live here; the GitHub side is EditorGithub.
//
// The repository is bound once, in mod.ltx (Mod Settings). The account is bound
// once per Windows user: a token in the Credential Manager, the GitHub CLI or an
// environment variable. A token is never written into the project, a log or an
// MCP answer; the one MCP request that carries one is save_token, which stores it.
//
// Everything the windows do, MCP does too, and the MCP path asks nothing: a
// caller that wants a permission asks for it on its own side.

namespace EditorPublish
{
	// opens the dialog - or, while a job runs, its progress
	ECORE_API void	RequestPublish	();
	ECORE_API bool	CanPublish		();
	// The Releases tab (a document, id "releases"): what the repository already
	// holds, drafts included, read again from GitHub; editing the title and the
	// notes of one, and deleting one together with its git tag. The tab asks
	// before it deletes and says that GitHub keeps no copy; MCP does not ask.
	ECORE_API void	RequestReleases	();
	// The factory the shell reopens a Releases tab of the session with; called
	// once at startup with the other document kinds, before a project opens.
	ECORE_API void	RegisterDocuments();
	// {"action":"list"|"edit"|"delete"|"status"|"cancel"|"select", "id"|"tag": which
	// release, "title"/"notes": what an edit sets}; list, edit and delete run on a
	// worker - the answer is pending until status says otherwise. select only marks
	// the release the Releases tab shows in its details; with "show":"edit" it also
	// puts the edit dialog up for it, to be looked at - only a click in it saves
	ECORE_API void	McpReleases		(LPCSTR raw, xr_string& out);
	// toolbar label: "Publish Release", or the progress of the running job
	ECORE_API void	ButtonText		(char* dst, u32 size);
	ECORE_API void	DrawUI			();
	// the editor is closing: cancels the job and the sign-in and waits for both
	ECORE_API void	Shutdown		();

	// {"action":"start"|"status"|"cancel"|"account"|"save_token"|"sign_in"|
	// "sign_in_status"|"sign_out", ...}; start answers as soon
	// as the release is packaged and the upload runs, status reports the rest
	ECORE_API void	McpPublish		(LPCSTR raw, xr_string& out);
}
