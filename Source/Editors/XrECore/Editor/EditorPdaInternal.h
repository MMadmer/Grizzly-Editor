#pragma once

// EditorPda's two halves share this: the model and MCP side (EditorPda.cpp) and the
// preview (EditorPdaPreview.cpp), which owns the game's textures and fonts.
namespace EditorPdaInternal
{
	// a texture id of textures_descr (the linked game's, the project's, the bundled PDA
	// sheet) or a texture file under textures\ of the game or the project
	bool	TextureKnown	(LPCSTR id);
	// a layout file was written or deleted: the preview re-reads what it shows
	void	FileWritten		(LPCSTR abs_path);
}
