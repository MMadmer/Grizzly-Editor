//------------------------------------------------------------------------------
// Textures for content browsed out of the linked game install.
//
// The game's archives are mounted in EditorGameContent's own registry, which the
// editor FS deliberately cannot see - that is what stops a game asset from ever
// shadowing an SDK one. The cost is that a model previewed straight from the
// game names textures the normal lookup cannot find. CRender::texture_load asks
// here after the editor's own folders and before its placeholder, and decodes
// the file itself: every mip, the format as it is, like any other texture.
//
// Read-only by construction: the only calls into the game install are Find and
// ReadBytes.
//------------------------------------------------------------------------------
#include "stdafx.h"
#pragma hdrstop

#include "EditorGameContent.h"
#include "EditorProject.h"

// `name` is an engine texture name: below textures\, no extension. The open
// project's own gamedata\textures\ comes first (the mod ships it in its module,
// over the game's), then what is mounted - the game is mounted by whatever
// brought the model in.
bool EditorGameContent_ReadTexture(LPCSTR name, xr_vector<u8>& file)
{
	file.clear();
	if (!name || !name[0])	return false;

	if (EditorProject::Active())
	{
		string_path	full;
		xr_sprintf	(full, sizeof(full), "%s\\gamedata\\textures\\%s.dds", EditorProject::Root(), name);
		for (char* c = full; *c; ++c) if (*c == '/') *c = '\\';
		if (FILE* f = nullptr; 0 == fopen_s(&f, full, "rb") && f)
		{
			fseek	(f, 0, SEEK_END);
			const long len = ftell(f);
			fseek	(f, 0, SEEK_SET);
			if (len > 0)
			{
				file.resize(size_t(len));
				if (fread(file.data(), 1, file.size(), f) != file.size())
					file.clear();
			}
			fclose	(f);
			if (!file.empty())	return true;
		}
	}

	// Find takes either slash and any case
	string_path	rel;
	xr_sprintf	(rel, sizeof(rel), "textures\\%s.dds", name);
	const int idx = EditorGameContent::Find(rel);
	if (idx < 0)	return false;

	u32	size	= 0;
	u8*	bytes	= EditorGameContent::ReadBytes(idx, size);
	if (bytes && size)
		file.assign(bytes, bytes + size);
	EditorGameContent::FreeBytes(bytes);
	return !file.empty();
}
