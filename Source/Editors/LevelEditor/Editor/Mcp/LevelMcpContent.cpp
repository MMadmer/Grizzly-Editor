#include "stdafx.h"
#include "LevelMcp.h"
#include "..\..\..\XrECore\Editor\EditorFileOps.h"
#include "..\..\..\XrECore\Editor\EditorGameContent.h"
#include "..\..\..\XrECore\Editor\EThumbnailVisual.h"
#include "..\..\..\XrECore\Editor\Nq\NqUtil.h"
#include "..\..\..\XrECore\Editor\EditorUiContract.h"

// The Content Browser and the files of the project over MCP: navigation, selection, copies, previews.
namespace LevelMcp
{
static bool IsBrowserTokenTerminator(LPCSTR cursor)
{
	while (*cursor == ' ' || *cursor == '\t' || *cursor == '\r' || *cursor == '\n') ++cursor;
	return *cursor == ',' || *cursor == '}';
}

// Browser enums accept either a complete JSON string token or a complete
// non-negative integer token. The caller applies the field-specific enum range.
static bool ParseBrowserToken(LPCSTR raw, LPCSTR field, char* parsed, u32 parsed_size,
	bool& present, bool& quoted)
{
	parsed[0] = 0;
	present = false;
	quoted = false;
	if (!raw) return true;

	char pattern[64];
	sprintf_s(pattern, "\"%s\"", field);
	LPCSTR key = strstr(raw, pattern);
	if (!key) return true;
	present = true;

	LPCSTR cursor = key + xr_strlen(pattern);
	while (*cursor == ' ' || *cursor == '\t' || *cursor == '\r' || *cursor == '\n') ++cursor;
	if (*cursor != ':') return false;
	for (++cursor; *cursor == ' ' || *cursor == '\t' || *cursor == '\r' || *cursor == '\n'; ++cursor) {}

	if (*cursor == '"')
	{
		quoted = true;
		LPCSTR start = ++cursor;
		for (; *cursor && *cursor != '"'; ++cursor)
		{
			if (*cursor != '\\') continue;
			if (!cursor[1]) return false;
			++cursor;
		}
		const size_t length = size_t(cursor - start);
		if (*cursor != '"' || length >= parsed_size || !IsBrowserTokenTerminator(cursor + 1))
			return false;
		return GrizzlyMCP::GetArg(raw, field, parsed, parsed_size);
	}

	LPCSTR start = cursor;
	while (*cursor >= '0' && *cursor <= '9') ++cursor;
	const size_t length = size_t(cursor - start);
	if (!length || length >= parsed_size || !IsBrowserTokenTerminator(cursor)) return false;
	CopyMemory(parsed, start, length);
	parsed[length] = 0;
	return true;
}

static bool ParseBrowserSource(LPCSTR raw, int& src)
{
	src = -1;
	char parsed[64];
	bool present = false;
	bool quoted = false;
	if (!ParseBrowserToken(raw, "source", parsed, sizeof(parsed), present, quoted)) return false;
	if (!present) return true;

	if (quoted)
	{
		if (0 == _stricmp(parsed, "project")) src = 0;
		else if (0 == _stricmp(parsed, "editor")) src = 1;
		else if (0 == _stricmp(parsed, "darf")) src = 2;
		else return false;
		return true;
	}

	if (parsed[1] || parsed[0] < '0' || parsed[0] > '2') return false;
	src = parsed[0] - '0';
	return true;
}

static bool ParseBrowserCategory(LPCSTR raw, int& category)
{
	category = -1;
	char parsed[64];
	bool present = false;
	bool quoted = false;
	if (!ParseBrowserToken(raw, "category", parsed, sizeof(parsed), present, quoted)) return false;
	if (!present) return true;

	if (quoted)
	{
		category = UIContentBrowser::FindCategory(parsed);
		return category >= 0 && category < UIContentBrowser::CategoryCount();
	}

	u32 value = 0;
	for (LPCSTR cursor = parsed; *cursor; ++cursor)
	{
		value = value * 10 + u32(*cursor - '0');
		if (value >= u32(UIContentBrowser::CategoryCount())) return false;
	}
	category = int(value);
	return true;
}

static LPCSTR BrowserSourceName(int src)
{
    switch (src)
    {
    case 0:		return "project";
    case 1:		return "editor";
    case 2:		return "darf";
    default:	return "";
    }
}

static void AppendBrowserLocationJson(xr_string& out, const UIContentBrowser::SLocation& location)
{
	char head[192];
	LPCSTR category = location.category >= 0 && location.category < UIContentBrowser::CategoryCount()
		? UIContentBrowser::CategoryName(location.category)
		: "";
	sprintf_s(head, "{\"source\":%d,\"source_name\":\"%s\",\"category\":%d,\"category_name\":\"%s\",\"folder\":\"",
		location.source, BrowserSourceName(location.source), location.category, category);
	out += head;
	out += JsonEscapePath(location.folder.c_str());
	out += "\"}";
}

// where the browser is seen - the docked panel on Level, the Content Drawer over any tab -, how that host shows the
// entries (tiles or the list) and the tile range (pt)
static void AppendBrowserHostJson(xr_string& out)
{
	// a panel just raised counts as seen: its tab comes to the front with the next frame
	const xr_string host = UIContentBrowser::VisibleHost();
	out += ",\"visible\":";
	out += host != "none" ? "true" : "false";
	out += ",\"host\":\"" + host + "\"";
	out += ",\"view\":\"";
	out += UIContentBrowser::ViewMode();
	out += "\"";
	char range[64];
	sprintf_s(range, ",\"tile_range\":[%.0f,%.0f],\"tile_unit\":\"pt\"", EditorUiContract::kTileMin, EditorUiContract::kTileMax);
	out += range;
}

// the view around the tiles: the search and the Sources pane (on = asked for, shown = it had the width)
static void AppendBrowserViewJson(xr_string& out)
{
	bool on = true, shown = false;
	UIContentBrowser::GetSourcesPane(on, shown);
	out += ",\"search\":\"";
	out += JsonEscapePath(UIContentBrowser::Search());
	out += "\",\"sources_pane\":{\"on\":";
	out += on ? "true" : "false";
	out += ",\"shown\":";
	out += shown ? "true" : "false";
	out += "}";
	// the toolbar's Add: on in the open project's own source only
	out += ",\"can_add\":";
	out += UIContentBrowser::CanAdd() ? "true" : "false";
}

static void RespondContentNavigationJson(xr_string& out)
{
	UIContentBrowser::SLocation current;
	xr_vector<UIContentBrowser::SLocation> back;
	xr_vector<UIContentBrowser::SLocation> forward;
	xr_vector<UIContentBrowser::SLocation> favorites;
	UIContentBrowser::GetHistory(current, back, forward);
	UIContentBrowser::GetFavorites(favorites);

	out = "{\"ok\":true,\"open\":";
	out += UIContentBrowser::IsOpen() ? "true" : "false";
	// the side the tiles are drawn at, and whether it is fitted to the host (no size picked)
	char tile[64];
	sprintf_s(tile, sizeof(tile), ",\"tile\":%.0f,\"tile_fit\":%s", UIContentBrowser::TileSize(),
		UIContentBrowser::TileFit() ? "true" : "false");
	out += tile;
	AppendBrowserHostJson(out);
	AppendBrowserViewJson(out);
	out += ",\"current\":";
	AppendBrowserLocationJson(out, current);
	const char* names[] = { "back", "forward", "favorites" };
	const xr_vector<UIContentBrowser::SLocation>* lists[] = { &back, &forward, &favorites };
	for (u32 list_index = 0; list_index < _countof(lists); ++list_index)
	{
		out += ",\"";
		out += names[list_index];
		out += "\":[";
		for (u32 i = 0; i < lists[list_index]->size(); ++i)
		{
			if (i)
				out += ",";
			AppendBrowserLocationJson(out, (*lists[list_index])[i]);
		}
		out += "]";
	}
	out += "}";
}

bool HandleContent(LPCSTR cmd, LPCSTR raw, xr_string& out)
{
    if (0 == xr_strcmp(cmd, "darf_list"))			{ EditorGameContent::McpList(raw, out);			return true; }
    if (0 == xr_strcmp(cmd, "darf_copy"))			{ EditorGameContent::McpCopy(raw, out);			return true; }

    // file management: writes and deletes are clamped to the project folder
    // inside EditorFileOps, which is where the path normalisation lives
    if (0 == xr_strcmp(cmd, "content_copy"))		{ EditorFileOps::McpCopy(raw, out);				return true; }
    if (0 == xr_strcmp(cmd, "content_move"))		{ EditorFileOps::McpMove(raw, out);				return true; }
    if (0 == xr_strcmp(cmd, "content_delete"))		{ EditorFileOps::McpDelete(raw, out);			return true; }
    if (0 == xr_strcmp(cmd, "content_mkdir"))		{ EditorFileOps::McpMakeDir(raw, out);			return true; }
    if (0 == xr_strcmp(cmd, "content_rename"))		{ EditorFileOps::McpRename(raw, out);			return true; }

	if (0 == xr_strcmp(cmd, "content_browser_navigation"))
	{
		char action[32] = {}, folder[512] = {};
		GrizzlyMCP::GetArg(raw, "action", action, sizeof(action));
		const bool has_folder = GrizzlyMCP::GetArg(raw, "folder", folder, sizeof(folder));
		if (!action[0])
			xr_strcpy(action, "get");

		int source = -1;
		if (!ParseBrowserSource(raw, source))
		{
			out = "{\"ok\":false,\"error\":\"source must be project, editor, darf or 0/1/2\"}";
			return true;
		}
		int category = -1;
		if (!ParseBrowserCategory(raw, category))
		{
			out = "{\"ok\":false,\"error\":\"unknown category (see list_assets)\"}";
			return true;
		}

		xr_string err;
		bool success = true;
		// the tile slider goes with any action
		float tile = 0.f;
		const bool tiled = GetArgFloat(raw, "tile", tile);
		// or none: tile_fit=true fits the tiles to the host again, false keeps the side they are shown at as picked
		const bool fitted = raw && strstr(raw, "\"tile_fit\"");
		const bool fit = fitted && GetArgBool(raw, "tile_fit", true);
		if (tiled && fit)
		{
			out = "{\"ok\":false,\"error\":\"tile picks a size: tile_fit=true goes without it\"}";
			return true;
		}
		if (tiled && !UIContentBrowser::SetTileSize(tile, err))
		{
			out = "{\"ok\":false,\"error\":\"";
			out += JsonEscapePath(err.c_str());
			out += "\"}";
			return true;
		}
		if (fitted && !tiled)
			UIContentBrowser::SetTileFit(fit);
		// so does the view (tiles or the list), after the size: a size asks for tiles
		char view[16] = {};
		const bool viewed = GrizzlyMCP::GetArg(raw, "view", view, sizeof(view));
		if (viewed && !UIContentBrowser::SetViewMode(view, err))
		{
			out = "{\"ok\":false,\"error\":\"";
			out += JsonEscapePath(err.c_str());
			out += "\"}";
			return true;
		}
		// so do the search field ("" clears it) and the Sources pane
		char search[256] = {};
		const bool searched = GrizzlyMCP::GetArg(raw, "search", search, sizeof(search));
		if (searched)
			UIContentBrowser::SetSearch(search);
		const bool paned = raw && strstr(raw, "\"sources_pane\"");
		if (paned)
			UIContentBrowser::SetSourcesPane(GetArgBool(raw, "sources_pane", true));
		if (0 == _stricmp(action, "navigate"))
			success = UIContentBrowser::NavigateToFolder(source, category, folder, err);
		else if (0 == _stricmp(action, "back"))
			success = UIContentBrowser::NavigateBack(err);
		else if (0 == _stricmp(action, "forward"))
			success = UIContentBrowser::NavigateForward(err);
		else if (0 == _stricmp(action, "up"))
			success = UIContentBrowser::NavigateUp(err);
		// the root of the source (and category) named, or of the one shown; already there is no failure
		else if (0 == _stricmp(action, "home"))
			success = source >= 0 || category >= 0
				? UIContentBrowser::NavigateToFolder(source, category, "", err)
				: UIContentBrowser::NavigateHome(err);
		else if (0 == _stricmp(action, "refresh"))
			UIContentBrowser::RefreshNow();
		else if (0 == _stricmp(action, "favorite_add"))
		{
			UIContentBrowser::SLocation current;
			xr_vector<UIContentBrowser::SLocation> back;
			xr_vector<UIContentBrowser::SLocation> forward;
			UIContentBrowser::GetHistory(current, back, forward);
			if (source >= 0)
				current.source = source;
			if (category >= 0)
				current.category = category;
			if (has_folder)
				current.folder = folder;
			success = UIContentBrowser::AddFavorite(current, err);
		}
		else if (0 == _stricmp(action, "favorite_remove"))
			success = UIContentBrowser::RemoveFavorite(GetArgInt(raw, "index", -1), err);
		else if (0 == _stricmp(action, "favorite_open"))
			success = UIContentBrowser::NavigateFavorite(GetArgInt(raw, "index", -1), err);
		else if (0 != _stricmp(action, "get"))
		{
			out = "{\"ok\":false,\"error\":\"action must be get, navigate, back, forward, up, home, refresh, favorite_add, favorite_remove, or favorite_open\"}";
			return true;
		}

		if (!success)
		{
			out = "{\"ok\":false,\"error\":\"";
			out += JsonEscapePath(err.c_str());
			out += "\"}";
			return true;
		}
		// moving through the folders, the tile size, the view, a search and the pane are visual: the browser comes up
		// where it can be seen - the host whose tile size or view was just set; get only reads
		if ((0 != _stricmp(action, "get") && 0 != _stricmp(action, "favorite_add") && 0 != _stricmp(action, "favorite_remove")) ||
			tiled || fitted || viewed || searched || paned)
			UIContentBrowser::MakeVisible();
		RespondContentNavigationJson(out);
		return true;
	}

    // reveal an asset in the content browser: the panel-driving twin of
    // asset_preview, which only ever renders offscreen
    if (0 == xr_strcmp(cmd, "content_browser_open"))
    {
        char name[512] = {};
        if (!GrizzlyMCP::GetArg(raw, "name", name, sizeof(name)) || !name[0])
        {
            out = "{\"ok\":false,\"error\":\"missing 'name' argument (see list_assets / darf_list)\"}";
            return true;
        }
        for (char* p = name; *p; ++p) if (*p == '/') *p = '\\';
        // GetArg hands over the raw JSON value, so an escaped "a\\b" arrives with
        // both slashes - the browser compares names exactly
        {
            char* w = name;
            for (const char* r = name; *r; ++r)
                if (!(*r == '\\' && w > name && w[-1] == '\\')) *w++ = *r;
            *w = 0;
        }

        int src = -1;
        if (!ParseBrowserSource(raw, src))
        {
            out = "{\"ok\":false,\"error\":\"source must be project, editor, darf or 0/1/2\"}";
            return true;
        }

        xr_string err;
        if (!UIContentBrowser::RevealAsset(name, src, GetArgBool(raw, "open", false), err))
        {
            out = "{\"ok\":false,\"error\":\"";
            out += JsonEscapePath(err.c_str());
            out += "\"}";
            return true;
        }

        // an opener: the docked panel on Level, the Content Drawer over anything else
        UIContentBrowser::MakeVisible();

        // report where it actually landed - RevealAsset may have switched source
        int cur = -1;
        xr_string folder;
        xr_vector<xr_string> sel;
        UIContentBrowser::GetSelection(cur, folder, sel);
        char head[128];
        sprintf_s(head, sizeof(head), "{\"ok\":true,\"source\":%d,\"source_name\":\"%s\",\"folder\":\"",
            cur, BrowserSourceName(cur));
        out = head;
        out += JsonEscapePath(folder.c_str());
        out += "\",\"name\":\"";
        out += JsonEscapePath(name);
        out += "\"";
        AppendBrowserHostJson(out);
        out += "}";
        return true;
    }

    // pull assets out of a read-only source into the project: the scriptable
    // twin of "Copy to project" / "Copy folder to project" in the browser menu
    if (0 == xr_strcmp(cmd, "content_browser_copy"))
    {
        char names[4096] = {}, folder[512] = {}, dst[512] = {}, cat[64] = {};
        GrizzlyMCP::GetArg(raw, "names",  names,  sizeof(names));
        GrizzlyMCP::GetArg(raw, "folder", folder, sizeof(folder));
        GrizzlyMCP::GetArg(raw, "dst",    dst,    sizeof(dst));
        GrizzlyMCP::GetArg(raw, "category", cat,  sizeof(cat));
        // GetArg hands over the raw JSON value, so an escaped "a\\b" arrives
        // with both slashes - the browser compares names exactly
        for (char* buf : { names, folder, dst })
        {
            for (char* p = buf; *p; ++p) if (*p == '/') *p = '\\';
            char* w = buf;
            for (const char* r = buf; *r; ++r)
                if (!(*r == '\\' && w > buf && w[-1] == '\\')) *w++ = *r;
            *w = 0;
        }

        int src = -1;
        if (!ParseBrowserSource(raw, src))
        {
            out = "{\"ok\":false,\"error\":\"source must be project, editor, darf or 0/1/2\"}";
            return true;
        }

        int category = -1;
        if (cat[0])
        {
            category = UIContentBrowser::FindCategory(cat);
            if (category < 0)
            {
                out = "{\"ok\":false,\"error\":\"unknown category (see list_assets)\"}";
                return true;
            }
        }

        int files = 0;
        xr_string err;
        const bool ok = UIContentBrowser::McpCopyToProject(
            names, folder, dst, src, category, GetArgBool(raw, "overwrite", true), files, err);

        char head[96];
        sprintf_s(head, sizeof(head), "{\"ok\":%s,\"files\":%d", ok ? "true" : "false", files);
        out = head;
        if (!err.empty())
        {
            out += ",\"error\":\"";
            out += JsonEscapePath(err.c_str());
            out += "\"";
        }
        out += "}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "content_create"))
    {
        char kind[32] = {};
        GrizzlyMCP::GetArg(raw, "kind", kind, sizeof(kind));
        xr_string made, err;
        if (!UIContentBrowser::CreateAsset(kind, made, err))
        {
            out = "{\"ok\":false,\"error\":\"";
            out += JsonEscapePath(err.c_str());
            out += "\"}";
            return true;
        }
        out = "{\"ok\":true,\"kind\":\"";
        out += JsonEscapePath(kind);
        out += "\",\"made\":\"";
        out += JsonEscapePath(made.c_str());
        out += "\"}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "content_browser_selection"))
    {
		char action[32] = {}, name[512] = {};
		GrizzlyMCP::GetArg(raw, "action", action, sizeof(action));
		GrizzlyMCP::GetArg(raw, "name", name, sizeof(name));
		// Show in Explorer: the entry selected in a window of Explorer; dry_run only answers the path
		if (0 == _stricmp(action, "reveal"))
		{
			xr_string path, err;
			if (!UIContentBrowser::RevealInExplorer(name, GetArgBool(raw, "dry_run", false), path, err))
			{
				out = "{\"ok\":false,\"error\":\"";
				out += JsonEscapePath(err.c_str());
				out += "\"}";
				return true;
			}
			out = "{\"ok\":true,\"path\":\"";
			out += JsonEscapePath(path.c_str());
			out += GetArgBool(raw, "dry_run", false) ? "\",\"opened\":false}" : "\",\"opened\":true}";
			return true;
		}
		if (action[0] && 0 != _stricmp(action, "get"))
		{
			xr_string err;
			if (!UIContentBrowser::ModifySelection(action, name, err))
			{
				out = "{\"ok\":false,\"error\":\"";
				out += JsonEscapePath(err.c_str());
				out += "\"}";
				return true;
			}
		}

        int src = -1;
        xr_string folder;
        xr_vector<xr_string> sel;
        UIContentBrowser::GetSelection(src, folder, sel);

        char head[192];
        sprintf_s(head, sizeof(head), "{\"ok\":true,\"open\":%s,\"source\":%d,\"source_name\":\"%s\",\"folder\":\"",
            UIContentBrowser::IsOpen() ? "true" : "false", src, BrowserSourceName(src));
        out = head;
        out += JsonEscapePath(folder.c_str());
        out += "\",\"count\":";
        char tail[32];
        sprintf_s(tail, sizeof(tail), "%u,\"selection\":[", u32(sel.size()));
        out += tail;
        for (u32 i = 0; i < sel.size(); ++i)
        {
            if (i) out += ",";
            out += "\"";
            out += JsonEscapePath(sel[i].c_str());
            out += "\"";
        }
        out += "]";
        // the drawn icon of every selected asset of the editor's own kinds, as its tile shows it
        out += ",\"icons\":{";
        bool first_icon = true;
        for (u32 i = 0; i < sel.size(); ++i)
        {
            LPCSTR icon = UIContentBrowser::IconName(sel[i].c_str());
            if (!icon[0]) continue;
            out += first_icon ? "\"" : ",\"";
            out += JsonEscapePath(sel[i].c_str());
            out += "\":\"";
            out += icon;
            out += "\"";
            first_icon = false;
        }
        out += "}";
        // the vector glyph of every selected entry, what its tile draws while it has no picture
        out += ",\"glyphs\":{";
        for (u32 i = 0; i < sel.size(); ++i)
        {
            out += i ? ",\"" : "\"";
            out += JsonEscapePath(sel[i].c_str());
            out += "\":\"";
            out += UIContentBrowser::Glyph(sel[i].c_str());
            out += "\"";
        }
        out += "}";
        // the inline rename box, while one is open: what it holds is what a hand would type over
        xr_string r_entry, r_text;
        int r_select = 0;
        if (UIContentBrowser::GetRename(r_entry, r_text, r_select))
        {
            out += ",\"rename\":{\"entry\":\"";
            out += JsonEscapePath(r_entry.c_str());
            out += "\",\"text\":\"";
            out += JsonEscapePath(r_text.c_str());
            sprintf_s(tail, sizeof(tail), "\",\"select\":%d}", r_select);
            out += tail;
        }
        out += "}";
        return true;
    }

    if (!Scene) return false;

    if (0 == xr_strcmp(cmd, "list_assets"))
    {
        char category[128] = {};
        int cat = 0; // Objects by default: the only placeable category
        if (GrizzlyMCP::GetArg(raw, "category", category, sizeof(category)) && category[0])
        {
            cat = UIContentBrowser::FindCategory(category);
            if (cat < 0)
            {
                out = "{\"ok\":false,\"error\":\"unknown category\",\"categories\":[";
                for (int i = 0; i < UIContentBrowser::CategoryCount(); ++i)
                {
                    if (i) out += ",";
                    out += "\"";
                    out += UIContentBrowser::CategoryName(i);
                    out += "\"";
                }
                out += "]}";
                return true;
            }
        }

        ChooseItemVec items;
        if (UIContentBrowser::IsLevelsCategory(cat))
        {
            // Levels are editor scenes, not library assets: they have no entry in
            // the choose-event table, which is why asking for them used to answer
            // "no enumerator". They come straight off the maps root instead -
            // plus the editor's own scene library, which the project remounts
            // $maps$ over and would otherwise hide completely.
            FS_FileSet lst;
            if (FS.file_list(lst, _maps_, FS_ListFiles | FS_ClampExt, "*.level"))
                for (FS_FileSetIt it = lst.begin(); it != lst.end(); ++it)
                    items.push_back(SChooseItem(it->name.c_str(), ""));
            xr_vector<xr_string> base;
            EditorProject::ListBaseScenes(base);
            for (u32 i = 0; i < base.size(); ++i)
            {
                bool have = false;
                for (u32 k = 0; k < items.size() && !have; ++k)
                    have = (0 == _stricmp(items[k].name.c_str(), base[i].c_str()));
                if (!have) items.push_back(SChooseItem(base[i].c_str(), ""));
            }
        }
        else
        {
            SChooseEvents* events = UIChooseForm::GetEvents(UIContentBrowser::CategoryId(cat));
            if (!events || !events->on_fill)
            {
                out = "{\"ok\":false,\"error\":\"category has no enumerator in this build\"}";
                return true;
            }
            events->on_fill(items, 0);
        }

        char filter[256] = {};
        GrizzlyMCP::GetArg(raw, "filter", filter, sizeof(filter));
        strlwr(filter);
        const int limit = GetArgInt(raw, "limit", 200);

        u32 total = 0, shown = 0;
        xr_string list;
        for (ChooseItemVecIt it = items.begin(); it != items.end(); ++it)
        {
            if (filter[0])
            {
                string512 lowered;
                strncpy_s(lowered, sizeof(lowered), it->name.c_str(), _TRUNCATE);
                strlwr(lowered);
                if (!strstr(lowered, filter)) continue;
            }
            ++total;
            if (int(shown) >= limit) continue;
            if (shown) list += ",";
            list += "{\"name\":\"";
            list += JsonEscapePath(it->name.c_str());
            list += "\"}";
            ++shown;
        }
        char head[256];
        sprintf_s(head, sizeof(head), "{\"ok\":true,\"category\":\"%s\",\"shown\":%u,\"total\":%u,\"assets\":[",
            UIContentBrowser::CategoryName(cat), shown, total);
        out = head;
        out += list;
        out += "]}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "asset_preview"))
    {
        char name[512];
        if (!GrizzlyMCP::GetArg(raw, "name", name, sizeof(name)))
        {
            out = "{\"ok\":false,\"error\":\"missing 'name' argument (see list_assets)\"}";
            return true;
        }
        for (char* p = name; *p; ++p) if (*p == '/') *p = '\\';
        // GetArg hands over the raw JSON value, so an escaped "a\\b" arrives with
        // both slashes. Lookups that compare paths exactly - the DARF registry
        // does a binary search - miss on that, even though the FS itself shrugs
        // doubled separators off.
        {
            char* w = name;
            for (const char* r = name; *r; ++r)
                if (!(*r == '\\' && w > name && w[-1] == '\\')) *w++ = *r;
            *w = 0;
        }

        // source=visual|darf render the model instead of looking up a baked .thm,
        // which is the only way to preview a game mesh: .ogf files ship no
        // thumbnail and are not part of the editor object library
        char source[64] = {};
        GrizzlyMCP::GetArg(raw, "source", source, sizeof(source));
        if (0 == _stricmp(source, "object"))
        {
            // library .object, rendered rather than read from a baked .thm
            U32Vec pixels;
            if (!RenderObjectThumbnail(name, pixels))
            {
                out = "{\"ok\":false,\"error\":\"cannot render this library object\"}";
                return true;
            }
            void* tex = GrizzlyMCP::PixelsToTexture(pixels);
            xr_string png;
            const bool encoded = tex && GrizzlyMCP::TextureToPngBase64(tex, png);
            GrizzlyMCP::ReleaseTexture(tex);
            if (!encoded)
            {
                out = "{\"ok\":false,\"error\":\"cannot encode the rendered preview\"}";
                return true;
            }
            out = "{\"ok\":true,\"rendered\":true,\"name\":\"";
            out += JsonEscapePath(name);
            out += "\",\"png_base64\":\"";
            out += png;
            out += "\"}";
            return true;
        }

        if (0 == _stricmp(source, "visual") || 0 == _stricmp(source, "darf"))
        {
            const bool from_game = (0 == _stricmp(source, "darf"));
            U32Vec pixels;
            bool rendered = false;

            if (from_game)
            {
                // mounting is lazy: without this the first DARF request of a
                // session reports "not found" against an empty registry
                xr_string mount_err;
                if (!EditorGameContent::EnsureMounted(mount_err))
                {
                    out = "{\"ok\":false,\"error\":\"";
                    out += mount_err;
                    out += "\"}";
                    return true;
                }
                const int idx = EditorGameContent::Find(name);
                if (idx < 0)
                {
                    out = "{\"ok\":false,\"error\":\"not found in the linked game install (see darf_list)\"}";
                    return true;
                }
                u32 sz = 0;
                u8* bytes = EditorGameContent::ReadBytes(idx, sz);
                if (bytes && sz)
                    rendered = RenderVisualThumbnailFromMemory(name, bytes, sz, pixels);
                EditorGameContent::FreeBytes(bytes);
                if (!rendered)
                {
                    // same fallback as the browser: render the editor's own copy
                    // of that mesh path when loading from bytes is unavailable
                    char visual[MAX_PATH];
                    LPCSTR src = name;
                    if (0 == _strnicmp(src, "meshes\\", 7)) src += 7;
                    strncpy_s(visual, src, _TRUNCATE);
                    if (char* dot = strrchr(visual, '.')) *dot = 0;
                    rendered = RenderVisualThumbnail(visual, pixels);
                }
            }
            else
            {
                rendered = RenderVisualThumbnail(name, pixels);
            }

            if (!rendered)
            {
                out = "{\"ok\":false,\"error\":\"cannot render a preview for this model\"}";
                return true;
            }
            void* tex = GrizzlyMCP::PixelsToTexture(pixels);
            if (!tex)
            {
                out = "{\"ok\":false,\"error\":\"cannot upload the rendered preview\"}";
                return true;
            }
            xr_string png;
            const bool encoded = GrizzlyMCP::TextureToPngBase64(tex, png);
            GrizzlyMCP::ReleaseTexture(tex);
            if (!encoded)
            {
                out = "{\"ok\":false,\"error\":\"cannot encode the rendered preview\"}";
                return true;
            }
            out = "{\"ok\":true,\"rendered\":true,\"name\":\"";
            out += JsonEscapePath(name);
            out += "\",\"png_base64\":\"";
            out += png;
            out += "\"}";
            return true;
        }

        char category[128] = {};
        int cat = 0;
        if (GrizzlyMCP::GetArg(raw, "category", category, sizeof(category)) && category[0])
        {
            cat = UIContentBrowser::FindCategory(category);
            if (cat < 0)
            {
                out = "{\"ok\":false,\"error\":\"unknown category\"}";
                return true;
            }
        }

        SChooseEvents* events = UIChooseForm::GetEvents(UIContentBrowser::CategoryId(cat));
        if (!events || !events->on_get_texture)
        {
            out = "{\"ok\":false,\"error\":\"category has no thumbnails in this build\"}";
            return true;
        }
        // the adapter allocates a fresh texture per call - release it after use
        ImTextureID tex = 0;
        events->on_get_texture(name, tex);
        if (!tex && UIContentBrowser::CategoryId(cat) == smVisual)
        {
            // the visual hook answers through a per-frame queue; a one-shot
            // MCP call cannot wait for it, so render right here instead
            U32Vec pixels;
            if (RenderVisualThumbnail(name, pixels))
                tex = static_cast<ImTextureID>(GrizzlyMCP::PixelsToTexture(pixels));
        }
        if (!tex)
        {
            out = "{\"ok\":false,\"error\":\"no preview for this asset (missing thumbnail or wrong name)\"}";
            return true;
        }
        xr_string png;
        const bool ok = GrizzlyMCP::TextureToPngBase64((void*)tex, png);
        GrizzlyMCP::ReleaseTexture((void*)tex);
        if (!ok)
        {
            out = "{\"ok\":false,\"error\":\"cannot encode the thumbnail\"}";
            return true;
        }
        out = "{\"ok\":true,\"name\":\"";
        out += JsonEscapePath(name);
        out += "\",\"png_base64\":\"";
        out += png;
        out += "\"}";
        return true;
    }

    return false;
}
}
