#pragma once

//------------------------------------------------------------------------------
// PDA layouts - a module's own PDA interface.
//
// A `.pdaasset` is declarative Lua, like a quest: the screens the PDA shows
// instead of its stock tabs - pages, buttons, lists, menus, and a slot for the
// live map. The module names the one it uses in mod.ltx:
//
//   [pda]
//   layout = pda\app.pdaasset
//
// and Dead Air Refined draws it inside the device (CUIPdaLayout in the game
// repo). The editor does not compile anything: the asset ships as is.
//
// What a layout MEANS is decided by one piece of Lua, the game's own
// scripts\xms_pda_layout.script. The editor runs that same file (from the
// linked game, the bundled copy without one) for its validator and its
// preview, so the two can never read a layout differently. The preview only
// draws the flat item list the reader returns - with the game's textures and
// the game's own bitmap fonts - and plays the actions the way the game does.
//
// Format: docs/pda/PDA_FORMAT.md. Game side: the game repo's MODDING.md,
// "PDA layouts".
//------------------------------------------------------------------------------

namespace EditorPda
{
	struct SProblem
	{
		xr_string	code;		// E3nn fatal for a build, W3nn a warning
		xr_string	path;		// where in the file: pages[2].widgets[1]
		xr_string	message;
	};

	// One item of the flat list the reader produces - the same fields the game's
	// CUIPdaLayout reads (canvas units, colours as ARGB).
	struct SItem
	{
		xr_string	kind;		// rect image text button list group map row
		xr_string	layer;		// chrome page overlay
		xr_string	page;
		int			parent;		// 1-based index; 0 = the layer
		xr_string	id;
		float		x, y, w, h;
		bool		visible;

		u32			color, hover, press, selected, border, tint;
		bool		has_hover, has_press, has_selected, has_border;
		float		radius, border_w;

		xr_string	texture;
		bool		stretch;

		xr_string	text, font, hint;
		bool		has_text;
		u32			text_color, text_hover, text_selected;
		bool		has_text_hover, has_text_selected;
		char		align, valign;
		bool		wrap;

		xr_string	icon;
		float		icon_size;
		u32			icon_color, icon_selected;
		bool		has_icon_selected;

		xr_vector<xr_string>	action;
		xr_vector<xr_string>	selected_on;

		float		row_h, gap;
		xr_string	empty;
		int			row;
		xr_string	zoom;

		SItem();
	};

	struct SPage
	{
		xr_string	id, title, stock;
	};

	struct SMarker
	{
		float		x, z;
		xr_string	icon, hint;
		u32			color;
		SMarker() : x(0), z(0), color(0xFFFFD21E) {}
	};

	struct SLayout
	{
		bool		ok;
		int			errors, warnings;
		xr_string	id, title, start;
		float		canvas_w, canvas_h;
		bool		noise, close;
		xr_string	key_map, key_tasks, key_contacts;
		xr_vector<std::pair<xr_string, xr_string> >	vars;
		xr_vector<SPage>		pages;
		xr_vector<SItem>		items;
		xr_vector<SProblem>		problems;

		// `preview = { ... }`: the editor's fake world, never read by the game
		xr_string	preview_level;
		bool		has_actor;
		float		actor_x, actor_z;
		xr_vector<std::pair<xr_string, xr_string> >	preview_vars;
		xr_vector<SMarker>		markers;

		SLayout();
		const SPage*	FindPage	(LPCSTR id) const;
	};

	// -- reading ---------------------------------------------------------------
	// Runs `text` through the game's reader. False + err (+ 1-based line, 0 when
	// unknown) when the file does not even parse; a file that parses always
	// comes back as a layout, with its coded problems.
	ECORE_API bool		Read		(LPCSTR text, u32 len, LPCSTR chunk, LPCSTR lang, SLayout& out,
									 xr_string& err, int& line);
	ECORE_API bool		ReadFile	(LPCSTR path, LPCSTR lang, SLayout& out, xr_string& err, int& line);
	// where the reader came from: "game:scripts\\xms_pda_layout.script" or "bundled:..."
	ECORE_API LPCSTR	ReaderSource();
	// A new layout: a starting point with the three pages every app has.
	ECORE_API xr_string	Template	(LPCSTR id, LPCSTR title);

	// -- the project -------------------------------------------------------------
	ECORE_API bool		IsPdaPath	(LPCSTR path);
	ECORE_API void		List		(LPCSTR project_root, xr_vector<xr_string>& files);	// absolute
	// mod.ltx [pda] layout, project-relative ("" = the module leaves the PDA alone)
	ECORE_API xr_string	UsedLayout	();
	// two project paths name one file ('/' and '\\' alike, case-blind)
	ECORE_API bool		SamePath	(LPCSTR a, LPCSTR b);
	// sets (rel) or clears ("") [pda] layout and saves mod.ltx at once
	ECORE_API bool		Use			(LPCSTR rel, xr_string& err);
	// The build's question: E-problems of the layout the module uses. A layout the
	// build leaves out, a missing file and one that does not parse are errors too.
	ECORE_API void		ValidateForBuild(LPCSTR project_root, xr_vector<xr_string>& errors,
									 xr_vector<xr_string>& warnings);
	// texture ids of a layout the linked game and the project do not have (W320)
	ECORE_API void		MissingTextures	(const SLayout& l, xr_vector<xr_string>& out);

	// -- MCP -------------------------------------------------------------------
	ECORE_API void		McpList		(LPCSTR raw, xr_string& out);
	ECORE_API void		McpGet		(LPCSTR raw, xr_string& out);
	ECORE_API void		McpWrite	(LPCSTR raw, xr_string& out);
	ECORE_API void		McpDelete	(LPCSTR raw, xr_string& out);
	ECORE_API void		McpUse		(LPCSTR raw, xr_string& out);
	ECORE_API void		McpValidate	(LPCSTR raw, xr_string& out);
	ECORE_API void		McpPreview	(LPCSTR raw, xr_string& out);

	// -- UI: the preview tab ---------------------------------------------------
	ECORE_API void		DrawUI		();
	ECORE_API void		Open		(LPCSTR path);		// from the content browser
	// Mod > PDA Interface: the layout the module uses; the project's first one; or a new
	// one from the template, made the module's layout
	ECORE_API void		ShowModuleLayout();
	// Create > PDA Interface of the content browser: a new file from the template in
	// `folder` (absolute), named NewPda, NewPda2, ...; false + err
	ECORE_API bool		CreateIn	(LPCSTR folder, string_path& created, xr_string& err);
	ECORE_API bool		IsOpen		();
	// the project or the link changed: textures, fonts and the reader are reloaded
	ECORE_API void		Invalidate	();
}
