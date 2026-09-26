#pragma once

//------------------------------------------------------------------------------
// Icons for the editor's own assets.
//
// A quest, a behaviour graph, a character and a PDA interface are files the
// editor invented, so nothing in the game ships a picture for them. They are
// drawn instead of loaded, by the editor's vector icon engine (XrEUI/XrUIIcons,
// docs/UI_DESIGN.md section 3): each kind is one XUI::Icon, built once and only
// scaled, exact at every tile size and DPI - where a bitmap is exact at one and
// blurred at the rest.
//
// Cost. Nothing here is recomputed per frame: the geometry lives in the engine,
// and what an asset IS costs one read of the first kilobyte of the file, cached
// per path. The cache is invalidated by generation, not by asking the disk: a
// tile draw never touches the file system, which is exactly the rule the
// startup profile taught (CLAUDE.md, "Startup").
//
// Call Invalidate() when the set of files may have changed - the content
// browser does it on refresh.
//------------------------------------------------------------------------------

struct ImDrawList;
namespace XUI { enum class Icon : int; }

namespace EditorAssetIcons
{
	enum EKind
	{
		kNone = 0,
		kQuest,				// .nqasset
		kBehaviour,			// .behasset
		kCharacter,			// .npcasset
		kCharacterTemplate,	// .npcasset with abstract = true
		kPda,				// .pdaasset
	};

	// What the file at `path` is. The extension answers it for a quest and a
	// behaviour graph; only a character has to be looked INTO, to tell a
	// template from the real thing (a cached peek at its head). kNone otherwise.
	ECORE_API EKind		KindOf		(LPCSTR path);
	// Drops the cached answers; the next KindOf for a path reads it again.
	ECORE_API void		Invalidate	();
	ECORE_API LPCSTR	Name		(EKind kind);
	// The extension of `name` when it is one of the editor's own asset kinds, 0
	// otherwise. For these the extension IS the kind and the icon already says it,
	// so the browser shows the file by its name alone and a rename never touches it.
	ECORE_API LPCSTR	OwnExtension(LPCSTR name);

	// The engine's icon for a kind; XUI::Icon::None for kNone.
	ECORE_API XUI::Icon	IconFor		(EKind kind);

	// Draws into the rectangle, centred at the size a tile has always shown it at,
	// on the CURRENT window's draw list, in the theme's text colour - the one that
	// reads on a plain tile and on a selected one alike, which is why a selected
	// tile does not recolour its icon any more than it recolours its label.
	ECORE_API void		Draw		(EKind kind, float x0, float y0, float x1, float y1);
	// The same on any draw list (a tab, a menu, a window that is not the current
	// one), in `color` when it is not 0.
	ECORE_API void		Draw		(ImDrawList* draw_list, EKind kind, float x0, float y0, float x1, float y1,
								 u32 color = 0);
	ECORE_API void		DrawFor		(LPCSTR path, float x0, float y0, float x1, float y1);
}
