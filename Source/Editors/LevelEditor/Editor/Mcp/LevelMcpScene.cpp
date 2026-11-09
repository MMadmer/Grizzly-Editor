#include "stdafx.h"
#include "LevelMcp.h"
#include "..\..\..\XrECore\Editor\Nq\NqUtil.h"
#include "..\..\..\XrECore\Editor\EditorGameContent.h"
#include "..\..\..\XrECore\Editor\UI_ViewportNavigation.h"
#include "..\..\..\..\xrServerEntities\xrServer_Objects_ALife_All.h"
#include "..\EditorModScene.h"
#include "..\GameLightingScene.h"
#include "..\..\..\XrECore\Editor\EGameLighting.h"
#include "..\Utils\Cursor3D.h"

// SpawnPoint.cpp: whether spawn objects draw their shapes, gizmos, icons and labels
extern bool g_SpawnHelpers;
// SceneObject.cpp: ray picks test every static object, seen by the camera or not
extern bool g_RayPickAll;

// The scene over MCP: objects, selection, placement, the camera and the viewport's view, the World Outliner.
namespace LevelMcp
{
static void AppendObjectJson(xr_string& out, CCustomObject* obj, LPCSTR cls)
{
    char tmp[512];
    const Fvector& p = obj->GetPosition();
    const Fvector& r = obj->GetRotation();
    const Fvector& s = obj->GetScale();
    out += "{\"name\":\"";
    out += JsonStr(obj->GetName());
    out += "\",\"class\":\"";
    out += JsonStr(cls);
    out += "\",";
    sprintf_s(tmp,
        "\"selected\":%s,\"visible\":%s,"
        "\"position\":[%.3f,%.3f,%.3f],\"rotation\":[%.4f,%.4f,%.4f],\"scale\":[%.3f,%.3f,%.3f]}",
        obj->Selected() ? "true" : "false",
        obj->Visible() ? "true" : "false",
        p.x, p.y, p.z, r.x, r.y, r.z, s.x, s.y, s.z);
    out += tmp;
}

static LPCSTR ResolveToolClassName(ObjClassID cls)
{
    for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
        if (it->second && it->first == cls) return it->second->ClassName();
    return "unknown";
}

static void AppendCameraJson(xr_string& out)
{
    char tmp[256];
    const Fvector& p = EDevice->m_Camera.GetPosition();
    const Fvector& r = EDevice->m_Camera.GetHPB();
    const Fvector& d = EDevice->m_Camera.GetDirection();
    sprintf_s(tmp,
        "\"camera\":{\"position\":[%.3f,%.3f,%.3f],\"hpb\":[%.4f,%.4f,%.4f],\"direction\":[%.3f,%.3f,%.3f]}",
        p.x, p.y, p.z, r.x, r.y, r.z, d.x, d.y, d.z);
    out += tmp;
}

// full single-object response shared by the object mutation commands
static void RespondObjectJson(xr_string& out, CCustomObject* obj)
{
    out = "{\"ok\":true,\"object\":";
    AppendObjectJson(out, obj, ResolveToolClassName(obj->FClassID));
    out += "}";
}

bool HandleScene(LPCSTR cmd, LPCSTR raw, xr_string& out)
{
	if (0 == xr_strcmp(cmd, "viewport_navigation"))
	{
		char action[32] = {}, view[32] = {}, scope[32] = {};
		GrizzlyMCP::GetArg(raw, "action", action, sizeof(action));
		GrizzlyMCP::GetArg(raw, "view", view, sizeof(view));
		GrizzlyMCP::GetArg(raw, "scope", scope, sizeof(scope));
		if (!action[0])
			xr_strcpy(action, view[0] ? "set" : "get");

		if (0 == _stricmp(action, "set"))
		{
			ViewportNavigation::Target target;
			if (!ViewportNavigation::TryParseTarget(view, target) || !ViewportNavigation::SetTarget(target))
			{
				out = "{\"ok\":false,\"error\":\"view must be perspective, front, back, left, right, top, or bottom\"}";
				return true;
			}
		}
		else if (0 == _stricmp(action, "frame"))
		{
			const bool selection = 0 == _stricmp(scope, "selection");
			if ((!selection && scope[0] && 0 != _stricmp(scope, "all")) ||
				!(selection ? ViewportNavigation::FrameSelection() : ViewportNavigation::FrameAll()))
			{
				out = "{\"ok\":false,\"error\":\"scope must be all or selection, and a scene must be ready\"}";
				return true;
			}
		}
		else if (0 != _stricmp(action, "get"))
		{
			out = "{\"ok\":false,\"error\":\"action must be get, set, or frame\"}";
			return true;
		}

		const ViewportNavigation::Target current = ViewportNavigation::CurrentTarget();
		out = "{\"ok\":true,\"view\":\"";
		out += ViewportNavigation::TargetKey(current);
		out += "\",";
		AppendCameraJson(out);
		out += "}";
		return true;
	}

    if (0 == xr_strcmp(cmd, "camera_get"))
    {
        out = "{\"ok\":true,";
        AppendCameraJson(out);
        out += "}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "camera_set"))
    {
        // absent fields keep their current value, so partial updates work
        Fvector pos = EDevice->m_Camera.GetPosition();
        Fvector hpb = EDevice->m_Camera.GetHPB();
        bool any = false;
        any |= GetArgFloat(raw, "x", pos.x);
        any |= GetArgFloat(raw, "y", pos.y);
        any |= GetArgFloat(raw, "z", pos.z);
        any |= GetArgFloat(raw, "h", hpb.x);
        any |= GetArgFloat(raw, "p", hpb.y);
        any |= GetArgFloat(raw, "b", hpb.z);
        if (!any)
        {
            out = "{\"ok\":false,\"error\":\"pass at least one of x,y,z,h,p,b\"}";
            return true;
        }
        EDevice->m_Camera.Set(hpb, pos);
		ViewportNavigation::ResetState();
        UI->RedrawScene(true);
        out = "{\"ok\":true,";
        AppendCameraJson(out);
        out += "}";
        return true;
    }

    // the outliner panel lives outside the scene, so it toggles without one
    if (0 == xr_strcmp(cmd, "outliner_show"))
    {
        // an opener: the Level tab comes up with the panel in front
        if (GetArgBool(raw, "open", true))	UIWorldOutliner::Focus(false);
        else								UIWorldOutliner::Close();
        out = UIWorldOutliner::IsOpen() ? "{\"ok\":true,\"open\":true}" : "{\"ok\":true,\"open\":false}";
        return true;
    }

    // the panel's search box and Filters menu, so a filtered view is scriptable; state only, the tab stays
    if (0 == xr_strcmp(cmd, "outliner_filter"))
    {
        char text[128] = {}, types[512] = {}, visibility[16] = {}, layout[16] = {}, sort[16] = {};
        const bool has_text  = GrizzlyMCP::GetArg(raw, "text",  text,  sizeof(text));
        const bool has_types = GrizzlyMCP::GetArg(raw, "types", types, sizeof(types));
        const bool has_visibility = GrizzlyMCP::GetArg(raw, "visibility", visibility, sizeof(visibility));
        const bool has_layout = GrizzlyMCP::GetArg(raw, "layout", layout, sizeof(layout)) && layout[0];
        const bool has_sort = GrizzlyMCP::GetArg(raw, "sort", sort, sizeof(sort)) && sort[0];
        int sel_only = -1;
        if (GetArgBool(raw, "selected_only", false))     sel_only = 1;
        else if (!GetArgBool(raw, "selected_only", true)) sel_only = 0;

        xr_string err;
        if (!UIWorldOutliner::McpSetFilter(has_text ? text : 0, sel_only,
                                           has_types ? types : 0,
                                           has_visibility ? visibility : 0,
                                           has_layout ? layout : 0, has_sort ? sort : 0, err))
        {
            out = "{\"ok\":false,\"error\":\"";
            out += JsonEscapePath(err.c_str());
            out += "\"}";
            return true;
        }

        xr_string cur, hidden, visibility_state, layout_state, sort_state;
        bool  only = false;
        int   shown = 0, total = 0;
        UIWorldOutliner::McpGetFilter(cur, only, hidden, visibility_state, layout_state, sort_state, shown, total);

        out = "{\"ok\":true,\"text\":\"";
        out += JsonEscapePath(cur.c_str());
        out += "\",\"selected_only\":";
        out += only ? "true" : "false";
        out += ",\"hidden_types\":\"";
        out += JsonEscapePath(hidden.c_str());
        char tail[256];
        sprintf_s(tail, "\",\"visibility\":\"%s\",\"layout\":\"%s\",\"sort\":\"%s\",\"shown\":%d,\"total\":%d}",
            visibility_state.c_str(), layout_state.c_str(), sort_state.c_str(), shown, total);
        out += tail;
        return true;
    }

    if (!Scene) return false;

    // What the eyes of the World Outliner and the Tools panel say: a class (the Tools tile and the Outliner's class row
    // share its flag), one object, the selection or everything. Without a target it only lists the classes.
    if (0 == xr_strcmp(cmd, "scene_visibility"))
    {
        char cls_name[64] = {}, name[256] = {};
        const bool has_class = GrizzlyMCP::GetArg(raw, "class", cls_name, sizeof(cls_name)) && cls_name[0];
        const bool has_name = GrizzlyMCP::GetArg(raw, "name", name, sizeof(name)) && name[0];
        const bool selection = GetArgBool(raw, "selection", false);
        const bool all = GetArgBool(raw, "all", false);
        const int targets = int(has_class) + int(has_name) + int(selection) + int(all);
        char visible_arg[16] = {};
        const bool has_visible = GrizzlyMCP::GetArg(raw, "visible", visible_arg, sizeof(visible_arg)) && visible_arg[0];
        const bool visible = GetArgBool(raw, "visible", true);
        if (targets > 1)
        {
            out = "{\"ok\":false,\"error\":\"pass one of class, name, selection=true or all=true\"}";
            return true;
        }
        if (targets && !has_visible)
        {
            out = "{\"ok\":false,\"error\":\"visible=true or visible=false says what to do\"}";
            return true;
        }

        int changed = 0;
        if (has_class)
        {
            ESceneToolBase* tool = nullptr;
            for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool() && !tool; ++it)
                if (it->second && it->first != OBJCLASS_DUMMY && 0 == _stricmp(it->second->ClassName(), cls_name))
                    tool = it->second;
            if (!tool)
            {
                out = "{\"ok\":false,\"error\":\"no class '";
                out += JsonEscapePath(cls_name);
                out += "': the classes are listed by scene_visibility without arguments\"}";
                return true;
            }
            if (!!tool->IsVisible() != visible)
            {
                tool->m_EditFlags.set(ESceneToolBase::flVisible, visible ? TRUE : FALSE);
                changed = 1;
            }
        }
        else if (has_name)
        {
            CCustomObject* obj = FindObjectRelaxed(name);
            if (!obj)
            {
                out = "{\"ok\":false,\"error\":\"no object '";
                out += JsonEscapePath(name);
                out += "'\"}";
                return true;
            }
            if (!!obj->Visible() != visible)
            {
                obj->Show(visible ? TRUE : FALSE);
                changed = 1;
            }
        }
        else if (targets)
            changed = UIWorldOutliner::ShowObjects(selection, visible);
        if (changed)
            UI->RedrawScene(true);

        char tmp[256];
        sprintf_s(tmp, "{\"ok\":true,\"changed\":%d,\"classes\":[", changed);
        out = tmp;
        bool first = true;
        for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
        {
            if (!it->second || it->first == OBJCLASS_DUMMY) continue;
            int objects = 0, hidden = 0;
            if (Scene->GetOTool(it->first))
            {
                ObjectList& lst = Scene->ListObj(it->first);
                for (ObjectIt o = lst.begin(); o != lst.end(); ++o)
                {
                    ++objects;
                    hidden += (*o)->Visible() ? 0 : 1;
                }
            }
            out += first ? "{\"class\":\"" : ",{\"class\":\"";
            first = false;
            out += JsonStr(it->second->ClassName());
            out += "\",\"title\":\"";
            out += JsonStr(ToolClassTitle(it->first));
            sprintf_s(tmp, "\",\"visible\":%s,\"objects\":%d,\"hidden_objects\":%d}",
                it->second->IsVisible() ? "true" : "false", objects, hidden);
            out += tmp;
        }
        out += "]}";
        return true;
    }

    // AI-facing twin of the World Outliner panel: one node per object tool
    if (0 == xr_strcmp(cmd, "scene_tree"))
    {
        char cls_filter[128] = {}, contains[128] = {};
        GrizzlyMCP::GetArg(raw, "class", cls_filter, sizeof(cls_filter));
        GrizzlyMCP::GetArg(raw, "filter", contains, sizeof(contains));
        const int limit = GetArgInt(raw, "limit", 200);
        strlwr(contains);

        // same gate and ordering as the panel: object tools only, by class name
        xr_vector<ESceneToolBase*> tools;
        for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
        {
            if (!it->second || it->first == OBJCLASS_DUMMY || !Scene->GetOTool(it->first)) continue;
            if (cls_filter[0] && 0 != _stricmp(it->second->ClassName(), cls_filter)) continue;
            if (Scene->ListObj(it->first).empty()) continue;
            tools.push_back(it->second);
        }
        std::sort(tools.begin(), tools.end(), [](ESceneToolBase* a, ESceneToolBase* b)
        {
            return xr_strcmp(a->ClassName(), b->ClassName()) < 0;
        });

        out = "{\"ok\":true,\"tree\":[";
        for (u32 g = 0; g < tools.size(); ++g)
        {
            ObjectList& lst = Scene->ListObj(tools[g]->FClassID);

            xr_string body;
            int matched = 0, emitted = 0;
            for (ObjectIt o = lst.begin(); o != lst.end(); ++o)
            {
                char low[256];
                strncpy_s(low, sizeof(low), (*o)->GetName(), _TRUNCATE);
                strlwr(low);
                if (contains[0] && !strstr(low, contains)) continue;
                ++matched;
                if (emitted >= limit) continue;
                char tmp[128];
                if (emitted) body += ",";
                body += "{\"name\":\"";
                body += JsonStr((*o)->GetName());
                sprintf_s(tmp, "\",\"selected\":%s,\"visible\":%s}",
                    (*o)->Selected() ? "true" : "false",
                    (*o)->Visible() ? "true" : "false");
                body += tmp;
                ++emitted;
            }
            if (!matched) continue;

            char head[256];
            sprintf_s(head, "%s{\"class\":\"%s\",\"count\":%d,\"total\":%d,\"objects\":[",
                (out[out.size() - 1] == '[') ? "" : ",", tools[g]->ClassName(), matched, int(lst.size()));
            out += head;
            out += body;
            out += "]}";
        }
        out += "]}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "list_objects"))
    {
        char cls_filter[128] = {}, contains[128] = {};
        GrizzlyMCP::GetArg(raw, "class", cls_filter, sizeof(cls_filter));
        GrizzlyMCP::GetArg(raw, "name_contains", contains, sizeof(contains));
        const int limit = GetArgInt(raw, "limit", 100);

        out = "{\"ok\":true,\"objects\":[";
        int emitted = 0, total = 0;
        for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
        {
            ESceneToolBase* t = it->second;
            if (!t || it->first == OBJCLASS_DUMMY || !Scene->GetOTool(it->first)) continue;
            if (cls_filter[0] && 0 != _stricmp(t->ClassName(), cls_filter)) continue;
            ObjectList& lst = Scene->ListObj(it->first);
            for (ObjectIt o = lst.begin(); o != lst.end(); ++o)
            {
                if (contains[0] && !strstr((*o)->GetName(), contains)) continue;
                ++total;
                if (emitted >= limit) continue;
                if (emitted) out += ",";
                out += "{\"name\":\"";
                out += JsonStr((*o)->GetName());
                out += "\",\"class\":\"";
                out += JsonStr(t->ClassName());
                out += "\"}";
                ++emitted;
            }
        }
        char tail[64];
        sprintf_s(tail, "],\"shown\":%d,\"total\":%d}", emitted, total);
        out += tail;
        return true;
    }

    if (0 == xr_strcmp(cmd, "focus_object"))
    {
        char name[256];
        if (!GrizzlyMCP::GetArg(raw, "name", name, sizeof(name)))
        {
            out = "{\"ok\":false,\"error\":\"missing 'name' argument\"}";
            return true;
        }
        CCustomObject* obj = FindObjectRelaxed(name);
        if (!obj)
        {
            out = "{\"ok\":false,\"error\":\"object not found\"}";
            return true;
        }

        if (GetArgBool(raw, "select", false))
        {
            // behave exactly like a manual pick: clear, select, switch target
            Scene->SelectObjects(false, OBJCLASS_DUMMY);
            obj->Select(TRUE);
            if (LTools->CurrentClassID() != obj->FClassID)
                LTools->SetTarget(obj->FClassID, 0);
        }

        // same math as the F key: frame the object's bounds, keep view direction
        Fbox box;
        if (!obj->GetBox(box))
        {
            const Fvector& p = obj->GetPosition();
            box.set(p, p);
            box.grow(2.f);
        }
        EDevice->m_Camera.ZoomExtents(box);
        UI->RedrawScene(true);	// forced frame so the RT is fresh even unfocused

        char tmp[256];
        const Fvector& cp = EDevice->m_Camera.GetPosition();
        sprintf_s(tmp, "{\"ok\":true,\"camera\":[%.2f,%.2f,%.2f]}", cp.x, cp.y, cp.z);
        out = tmp;
        return true;
    }

    if (0 == xr_strcmp(cmd, "scene_info"))
    {
        char tmp[512];
        // the scene name is a path once a level is loaded - raw backslashes
        // break the JSON on the bridge side
        sprintf_s(tmp, "{\"ok\":true,\"scene\":\"%s\",\"modified\":%s,\"tools\":[",
            JsonEscapePath(Tools ? Tools->m_LastFileName.c_str() : "").c_str(),
            UI->IsModified() ? "true" : "false");
        out = tmp;
        bool first = true;
        for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
        {
            ESceneToolBase* t = it->second;
            if (!t || it->first == OBJCLASS_DUMMY) continue;
            // AI map & co are not object tools — ListObj/ObjCount would assert
            const int cnt = Scene->GetOTool(it->first) ? Scene->ObjCount(it->first) : 0;
            sprintf_s(tmp, "%s{\"class\":\"%s\",\"objects\":%d}",
                first ? "" : ",", t->ClassName(), cnt);
            out += tmp;
            first = false;
        }
        out += "]}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "selection"))
    {
        out = "{\"ok\":true,\"selection\":[";
        bool first = true;
        for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
        {
            ESceneToolBase* t = it->second;
            if (!t || it->first == OBJCLASS_DUMMY || !Scene->GetOTool(it->first)) continue;
            ObjectList& lst = Scene->ListObj(it->first);
            for (ObjectIt o = lst.begin(); o != lst.end(); ++o)
            {
                if (!(*o)->Selected()) continue;
                if (!first) out += ",";
                AppendObjectJson(out, *o, t->ClassName());
                first = false;
            }
        }
        out += "]}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "object_info"))
    {
        char name[256];
        if (!GrizzlyMCP::GetArg(raw, "name", name, sizeof(name)))
        {
            out = "{\"ok\":false,\"error\":\"missing 'name' argument\"}";
            return true;
        }
        CCustomObject* obj = FindObjectRelaxed(name);
        if (!obj)
        {
            out = "{\"ok\":false,\"error\":\"object not found\"}";
            return true;
        }
        RespondObjectJson(out, obj);
        return true;
    }

    if (0 == xr_strcmp(cmd, "scene_stats"))
    {
        char tmp[512];
        // selection spans every object tool, not just the active target class
        int selected = 0;
        for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
        {
            if (!it->second || it->first == OBJCLASS_DUMMY || !Scene->GetOTool(it->first)) continue;
            ObjectList& lst = Scene->ListObj(it->first);
            for (ObjectIt o = lst.begin(); o != lst.end(); ++o)
                if ((*o)->Selected()) ++selected;
        }
        // path-escaped for the same reason as scene_info: once a level is
        // loaded the name carries backslashes
        sprintf_s(tmp, "{\"ok\":true,\"scene\":\"%s\",\"modified\":%s,\"selected\":%d,",
            JsonEscapePath(Tools ? Tools->m_LastFileName.c_str() : "").c_str(),
            UI->IsModified() ? "true" : "false", selected);
        out = tmp;
        AppendCameraJson(out);
        out += ",\"tools\":[";
        bool first = true;
        for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
        {
            ESceneToolBase* t = it->second;
            if (!t || it->first == OBJCLASS_DUMMY) continue;
            const int cnt = Scene->GetOTool(it->first) ? Scene->ObjCount(it->first) : 0;
            sprintf_s(tmp, "%s{\"class\":\"%s\",\"objects\":%d}",
                first ? "" : ",", t->ClassName(), cnt);
            out += tmp;
            first = false;
        }
        out += "]}";
        return true;
    }

    // {"authored":true|false} - flips the mod-content flag on the selection;
    // the build ships only authored objects (imported base levels stay base)
    if (0 == xr_strcmp(cmd, "mark_authored"))
    {
        if (!Scene) { out = "{\"ok\":false,\"error\":\"no scene\"}"; return true; }
        const bool authored = GetArgBool(raw, "authored", true);
        int n = 0;
        ObjectList sel;
        if (Scene->GetQueryObjects(sel, OBJCLASS_DUMMY, 1, -1, -1))
            for (ObjectIt it = sel.begin(); it != sel.end(); ++it)
                if (!!(*it)->IsAuthorPlaced() != authored)
                    { (*it)->SetAuthorPlaced(authored ? TRUE : FALSE); ++n; }
        if (n) Scene->UndoSave();
        char buf[96];
        sprintf_s(buf, "{\"ok\":true,\"changed\":%d,\"authored\":%s}", n, authored ? "true" : "false");
        out = buf;
        return true;
    }

    // a base level's spawn the mod takes out of the game (the build writes a remove op);
    // authored objects are the mod's own and are simply deleted instead
    if (0 == xr_strcmp(cmd, "mark_removed"))
    {
        if (!Scene) { out = "{\"ok\":false,\"error\":\"no scene\"}"; return true; }
        const bool removed = GetArgBool(raw, "removed", true);
        int n = 0, own = 0;
        ObjectList sel;
        if (Scene->GetQueryObjects(sel, OBJCLASS_SPAWNPOINT, 1, -1, -1))
            for (ObjectIt it = sel.begin(); it != sel.end(); ++it)
            {
                if ((*it)->IsAuthorPlaced()) { ++own; continue; }
                if (!!(*it)->IsBaseRemoved() != removed)
                    { (*it)->SetBaseRemoved(removed ? TRUE : FALSE); ++n; }
            }
        if (n) Scene->UndoSave();
        char buf[128];
        sprintf_s(buf, "{\"ok\":true,\"changed\":%d,\"skipped_authored\":%d,\"removed\":%s}", n, own, removed ? "true" : "false");
        out = buf;
        return true;
    }

    if (0 == xr_strcmp(cmd, "select_objects"))
    {
        char names[2048];
        if (!GrizzlyMCP::GetArg(raw, "names", names, sizeof(names)))
        {
            out = "{\"ok\":false,\"error\":\"missing 'names' argument (comma-separated list)\"}";
            return true;
        }
        Scene->SelectObjects(false, OBJCLASS_DUMMY);
        int found = 0;
        ObjClassID first_cls = OBJCLASS_DUMMY;
        xr_string missing = "[";
        char* ctx = 0;
        for (char* tok = strtok_s(names, ",", &ctx); tok; tok = strtok_s(0, ",", &ctx))
        {
            while (*tok == ' ') ++tok;
            for (char* e = tok + xr_strlen(tok); e > tok && e[-1] == ' ';) *--e = 0;
            if (!*tok) continue;
            CCustomObject* obj = FindObjectRelaxed(tok);
            if (obj)
            {
                obj->Select(TRUE);
                if (!found) first_cls = obj->FClassID;
                ++found;
            }
            else
            {
                if (missing.size() > 1) missing += ",";
                missing += "\"";
                missing += tok;
                missing += "\"";
            }
        }
        missing += "]";
        // behave like a manual pick: the first found class becomes the target
        if (found && LTools->CurrentClassID() != first_cls)
            LTools->SetTarget(first_cls, 0);
        UI->RedrawScene(true);
        char tmp[64];
        sprintf_s(tmp, "{\"ok\":true,\"selected\":%d,\"missing\":", found);
        out = tmp;
        out += missing;
        out += "}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "deselect_all"))
    {
        Scene->SelectObjects(false, OBJCLASS_DUMMY);
        UI->RedrawScene(true);
        out = "{\"ok\":true}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "object_set_position") ||
        0 == xr_strcmp(cmd, "object_set_rotation") ||
        0 == xr_strcmp(cmd, "object_set_scale"))
    {
        char name[256];
        if (!GrizzlyMCP::GetArg(raw, "name", name, sizeof(name)))
        {
            out = "{\"ok\":false,\"error\":\"missing 'name' argument\"}";
            return true;
        }
        CCustomObject* obj = FindObjectRelaxed(name);
        if (!obj)
        {
            out = "{\"ok\":false,\"error\":\"object not found\"}";
            return true;
        }
        const bool set_rot = (0 == xr_strcmp(cmd, "object_set_rotation"));
        const bool set_scl = (0 == xr_strcmp(cmd, "object_set_scale"));
        // absent components keep their current value
        Fvector v = set_rot ? obj->GetRotation() : set_scl ? obj->GetScale() : obj->GetPosition();
        bool any = false;
        any |= GetArgFloat(raw, "x", v.x);
        any |= GetArgFloat(raw, "y", v.y);
        any |= GetArgFloat(raw, "z", v.z);
        if (!any)
        {
            out = "{\"ok\":false,\"error\":\"pass at least one of x,y,z\"}";
            return true;
        }
        if (!_valid(v))
        {
            out = "{\"ok\":false,\"error\":\"non-finite component\"}";
            return true;
        }
		ObjectList transformed;
		transformed.push_back(obj);
		Scene->UndoTransformBegin(transformed);
		if      (set_rot)	obj->SetRotation(v);	// radians
        else if (set_scl)	obj->SetScale(v);
        else				obj->SetPosition(v);
		Scene->UndoTransformEnd();
        // the properties panel caches the transform on selection - without this
        // it keeps showing the pre-command values
        ExecCommand(COMMAND_UPDATE_PROPERTIES);
        UI->RedrawScene(true);
        RespondObjectJson(out, obj);
        return true;
    }

    if (0 == xr_strcmp(cmd, "place_object"))
    {
        if (Scene->locked())
        {
            out = "{\"ok\":false,\"error\":\"scene locked\"}";
            return true;
        }
        char ref[256];
        if (!GrizzlyMCP::GetArg(raw, "ref", ref, sizeof(ref)))
        {
            out = "{\"ok\":false,\"error\":\"missing 'ref' argument (library object, e.g. props\\\\box_1)\"}";
            return true;
        }
        for (char* p = ref; *p; ++p) if (*p == '/') *p = '\\';

        // full transform: position, rotation (radians), scale. Anything the
        // caller omits keeps its default; with no position at all the object
        // still lands in front of the camera so a bare place_object works.
        Fvector pos;
        pos.mad(EDevice->m_Camera.GetPosition(), EDevice->m_Camera.GetDirection(), 10.f);
        bool explicit_pos = false;
        explicit_pos |= GetArgFloat(raw, "x", pos.x);
        explicit_pos |= GetArgFloat(raw, "y", pos.y);
        explicit_pos |= GetArgFloat(raw, "z", pos.z);

        Fvector rot; rot.set(0.f, 0.f, 0.f);
        bool explicit_rot = false;
        explicit_rot |= GetArgFloat(raw, "rx", rot.x);
        explicit_rot |= GetArgFloat(raw, "ry", rot.y);
        explicit_rot |= GetArgFloat(raw, "rz", rot.z);

        Fvector scale; scale.set(1.f, 1.f, 1.f);
        bool explicit_scale = false;
        explicit_scale |= GetArgFloat(raw, "sx", scale.x);
        explicit_scale |= GetArgFloat(raw, "sy", scale.y);
        explicit_scale |= GetArgFloat(raw, "sz", scale.z);
        // uniform shorthand: scale = <f> sets all three
        float uniform_scale;
        if (GetArgFloat(raw, "scale", uniform_scale))
        {
            scale.set(uniform_scale, uniform_scale, uniform_scale);
            explicit_scale = true;
        }

        if (!_valid(pos) || !_valid(rot) || !_valid(scale))
        {
            out = "{\"ok\":false,\"error\":\"non-finite transform component\"}";
            return true;
        }
        if (explicit_scale && (fis_zero(scale.x) || fis_zero(scale.y) || fis_zero(scale.z)))
        {
            out = "{\"ok\":false,\"error\":\"zero scale component\"}";
            return true;
        }

        string256 namebuffer;
        char custom[256] = {};
        if (GrizzlyMCP::GetArg(raw, "name", custom, sizeof(custom)) && custom[0])
        {
            strlwr(custom);
            if (Scene->FindObjectByName(custom, (CCustomObject*)0))
            {
                out = "{\"ok\":false,\"error\":\"object name already used\"}";
                return true;
            }
            strncpy_s(namebuffer, sizeof(namebuffer), custom, _TRUNCATE);
        }
        else
            Scene->GenObjectName(OBJCLASS_SCENEOBJECT, namebuffer, ref);

        CSceneObject* obj = xr_new<CSceneObject>((LPVOID)0, namebuffer);
        if (!obj->SetReference(ref))
        {
            xr_delete(obj);
            out = "{\"ok\":false,\"error\":\"can't load reference object (check the library path)\"}";
            return true;
        }
        obj->SetPosition(pos);
        if (explicit_rot)   obj->SetRotation(rot);
        if (explicit_scale) obj->SetScale(scale);

        if (LTools->CurrentClassID() != OBJCLASS_SCENEOBJECT)
            LTools->SetTarget(OBJCLASS_SCENEOBJECT, 0);
        Scene->SelectObjects(false, OBJCLASS_SCENEOBJECT);
        Scene->AppendObject(obj);

        // optional: settle it on the surface below, like the End-key drop
        bool dropped = false;
        if (GetArgBool(raw, "snap_to_ground", false))
        {
            ObjectList batch;
            batch.push_back(obj);
            dropped = DropObjectsToGround(batch, GetArgBool(raw, "align_normal", false), false) > 0;
        }

        UI->RedrawScene(true);
        RespondObjectJson(out, obj);
        // splice the drop result in before the closing brace
        if (!out.empty() && out[out.size() - 1] == '}')
        {
            out.erase(out.size() - 1);
            out += dropped ? ",\"snapped_to_ground\":true}" : ",\"snapped_to_ground\":false}";
        }
        return true;
    }

    // ---- gameplay objects: the spawn roster and spawn placement ------------
    // The roster IS pSettings (every $spawn section of the LINKED game once a
    // project is linked), which is exactly what the old SDK's "Spawn Element"
    // panel showed. list first, place by section - the entity, its shapes
    // ($def_sphere) and its properties come from the section itself.
    if (0 == xr_strcmp(cmd, "list_spawn_sections"))
    {
        char filter[256] = {};
        GrizzlyMCP::GetArg(raw, "filter", filter, sizeof(filter));
        strlwr(filter);
        const int limit = GetArgInt(raw, "limit", 200);

        int total = 0, shown = 0;
        out = "{\"ok\":true,\"sections\":[";
        if (pSettings)
        {
            CInifile::Root const& data = pSettings->sections();
            for (CInifile::RootCIt it = data.begin(); it != data.end(); ++it)
            {
                LPCSTR val;
                if (!(*it)->line_exist("$spawn", &val))                 continue;
                if (!pSettings->line_exist((*it)->Name, "class"))       continue;
                // the path the Spawn tool's list shows (a stray indent or ';' in the game's value dropped)
                const xr_string group = UISpawnTool::ListPath(pSettings->r_string_wb((*it)->Name, "$spawn").c_str());
                if (filter[0])
                {
                    string512 hay;
                    xr_sprintf(hay, "%s %s", (*it)->Name.c_str(), group.c_str());
                    strlwr(hay);
                    if (!strstr(hay, filter))                           continue;
                }
                ++total;
                if (shown >= limit)                                     continue;
                if (shown) out += ",";
                out += "{\"section\":\"";
                EditorGameContent::JsonAppendPath(out, (*it)->Name.c_str());
                out += "\",\"group\":\"";
                EditorGameContent::JsonAppendPath(out, group.c_str());
                out += "\",\"class\":\"";
                EditorGameContent::JsonAppendPath(out, pSettings->r_string((*it)->Name, "class"));
                out += "\"}";
                ++shown;
            }
        }
        char tail[96];
        sprintf_s(tail, "],\"shown\":%d,\"total\":%d}", shown, total);
        out += tail;
        return true;
    }

    if (0 == xr_strcmp(cmd, "place_spawn"))
    {
        if (Scene->locked())
        {
            out = "{\"ok\":false,\"error\":\"scene locked\"}";
            return true;
        }
        char section[256];
        if (!GrizzlyMCP::GetArg(raw, "section", section, sizeof(section)))
        {
            out = "{\"ok\":false,\"error\":\"missing 'section' argument (see list_spawn_sections)\"}";
            return true;
        }
        // The points the Spawn tool's list names itself are no config sections: the module's Player Start, an
        // Environment Modifier, a Respawn Point. Asked for by their factory name or without its '$'.
        static const LPCSTR kEditorPoints[] = { PLAYERSTART_CHOOSE_NAME, ENVMOD_CHOOSE_NAME, RPOINT_CHOOSE_NAME };
        LPCSTR editor_point = nullptr;
        for (LPCSTR point : kEditorPoints)
            if (0 == _stricmp(section, point) || 0 == _stricmp(section, point + 1))
                editor_point = point;
        if (editor_point)
            xr_strcpy(section, editor_point);
        else if (!pSettings || !pSettings->section_exist(section) || !pSettings->line_exist(section, "$spawn"))
        {
            out = "{\"ok\":false,\"error\":\"no such spawn section in pSettings (see list_spawn_sections; player_start, env_mod and "
                  "rpoint place the Player Start, an Environment Modifier and a Respawn Point)\"}";
            return true;
        }

        Fvector pos;
        pos.mad(EDevice->m_Camera.GetPosition(), EDevice->m_Camera.GetDirection(), 10.f);
        GetArgFloat(raw, "x", pos.x);
        GetArgFloat(raw, "y", pos.y);
        GetArgFloat(raw, "z", pos.z);
        if (!_valid(pos))
        {
            out = "{\"ok\":false,\"error\":\"non-finite position\"}";
            return true;
        }

        ESceneCustomOTool* tool = Scene->GetOTool(OBJCLASS_SPAWNPOINT);
        if (!tool)
        {
            out = "{\"ok\":false,\"error\":\"no spawn tool in this build\"}";
            return true;
        }

        string256 namebuffer;
        // "$env_mod" names the factory, not the object (the Spawn tool names it the same way)
        Scene->GenObjectName(OBJCLASS_SPAWNPOINT, namebuffer, editor_point ? editor_point + 1 : section);
        // the tool's own factory: builds the server entity from the section
        // and auto-attaches whatever the section declares ($def_sphere shapes)
        CCustomObject* obj = tool->CreateObject((LPVOID)section, namebuffer);
        if (!obj)
        {
            out = "{\"ok\":false,\"error\":\"the spawn factory refused this section\"}";
            return true;
        }
        obj->SetPosition(pos);

        if (LTools->CurrentClassID() != OBJCLASS_SPAWNPOINT)
            LTools->SetTarget(OBJCLASS_SPAWNPOINT, 0);
        Scene->SelectObjects(false, OBJCLASS_SPAWNPOINT);
        Scene->AppendObject(obj);

        bool dropped = false;
        if (GetArgBool(raw, "snap_to_ground", false))
        {
            ObjectList batch;
            batch.push_back(obj);
            dropped = DropObjectsToGround(batch, false, false) > 0;
        }

        UI->RedrawScene(true);
        RespondObjectJson(out, obj);
        if (!out.empty() && out[out.size() - 1] == '}')
        {
            out.erase(out.size() - 1);
            out += dropped ? ",\"snapped_to_ground\":true}" : ",\"snapped_to_ground\":false}";
        }
        return true;
    }

    // A way path, drawn from points: what a patrol walks, a post watches and a quest's
    // destination names. The WayPoint tool draws the same thing by hand.
    if (0 == xr_strcmp(cmd, "place_way"))
    {
        if (Scene->locked())
        {
            out = "{\"ok\":false,\"error\":\"scene locked\"}";
            return true;
        }
        char name[256];
        xr_string text;
        if (!GrizzlyMCP::GetArg(raw, "name", name, sizeof(name)) || !name[0])
        {
            out = "{\"ok\":false,\"error\":\"missing 'name' - the path's name, what orders and quests call it\"}";
            return true;
        }
        if (!NqUtil::ArgString(raw, "points", text))
        {
            out = "{\"ok\":false,\"error\":\"missing 'points': \\\"x,y,z; x,y,z; ...\\\"\"}";
            return true;
        }
        if (Scene->FindObjectByName(name, (CCustomObject*)0))
        {
            out = "{\"ok\":false,\"error\":\"an object of that name exists already\"}";
            return true;
        }
        xr_vector<Fvector> points;
        xr_vector<xr_string> items;
        NqUtil::Split(text.c_str(), ';', items);
        for (const xr_string& item : items)
        {
            Fvector p;
            if (3 != sscanf(item.c_str(), "%f , %f , %f", &p.x, &p.y, &p.z) || !_valid(p))
            {
                out = "{\"ok\":false,\"error\":\"a point is not x,y,z: ";
                out += JsonStr(item.c_str());
                out += "\"}";
                return true;
            }
            points.push_back(p);
        }
        if (points.empty())
        {
            out = "{\"ok\":false,\"error\":\"a path needs at least one point\"}";
            return true;
        }
        // each point onto what is under it, the way a hand-drawn point lands
        if (GetArgBool(raw, "snap_to_ground", true))
            for (Fvector& p : points)
            {
                Fvector start = p, down;
                start.y += 2.f;
                down.set(0.f, -1.f, 0.f);
                SRayPickInfo pinf;
                pinf.inf.range = kDropReach;
                if (Scene->RayPickObject(pinf.inf.range, start, down, OBJCLASS_SCENEOBJECT, &pinf, 0))
                {
                    Fvector hit;
                    RetrieveSceneObjPointAndNormal(hit, 0, pinf, 0);
                    if (_valid(hit)) p = hit;
                }
            }
        ESceneCustomOTool* tool = Scene->GetOTool(OBJCLASS_WAY);
        CWayObject* way = tool ? dynamic_cast<CWayObject*>(tool->CreateObject(0, name)) : 0;
        if (!way)
        {
            out = "{\"ok\":false,\"error\":\"no way tool in this build\"}";
            return true;
        }
        way->BuildPath(points, GetArgBool(raw, "loop", true));
        Scene->SelectObjects(false, OBJCLASS_WAY);
        Scene->AppendObject(way);
        UI->RedrawScene(true);
        RespondObjectJson(out, way);
        if (!out.empty() && out[out.size() - 1] == '}')
        {
            out.erase(out.size() - 1);
            char tail[64];
            sprintf_s(tail, ",\"points\":%u}", (u32)points.size());
            out += tail;
        }
        return true;
    }

    // A placed stalker's standing order: what it does from its first frame in the game
    // until a quest says otherwise. Without order fields the answer only reports it.
    if (0 == xr_strcmp(cmd, "spawn_order"))
    {
        char name[256];
        if (!GrizzlyMCP::GetArg(raw, "name", name, sizeof(name)))
        {
            out = "{\"ok\":false,\"error\":\"missing 'name' (a stalker placed with the Spawn tool)\"}";
            return true;
        }
        CSpawnPoint* sp = dynamic_cast<CSpawnPoint*>(FindObjectRelaxed(name));
        if (!sp)
        {
            out = "{\"ok\":false,\"error\":\"no spawn object of that name\"}";
            return true;
        }
        if (!sp->CanTakeOrder())
        {
            out = "{\"ok\":false,\"error\":\"only a stalker takes an order - the game's schemes drive stalkers alone\"}";
            return true;
        }
        SXmsOrder o = sp->m_Order;
        bool changed = false;
        char text[256];
        if (GrizzlyMCP::GetArg(raw, "order", text, sizeof(text)))
        {
            const u8 kind = SXmsOrder::KindFromKey(text);
            if (kind == SXmsOrder::okNone && 0 != _stricmp(text, "none"))
            {
                out = "{\"ok\":false,\"error\":\"order must be guard, patrol, wander, follow or none\"}";
                return true;
            }
            o.kind = kind;
            changed = true;
        }
        if (GrizzlyMCP::GetArg(raw, "route", text, sizeof(text)))	{ o.route = text; changed = true; }
        if (GrizzlyMCP::GetArg(raw, "look", text, sizeof(text)))		{ o.look = text; changed = true; }
        if (GrizzlyMCP::GetArg(raw, "target", text, sizeof(text)))	{ o.target = text; changed = true; }
        float number = 0.f;
        if (GetArgFloat(raw, "radius", number))		{ o.radius = number; changed = true; }
        if (GetArgFloat(raw, "distance", number))	{ o.distance = number; changed = true; }
        // a flag is set only when the request names it: asked with both defaults, an
        // absent field answers twice differently
        auto flag = [&](LPCSTR field, u8 mask)
        {
            const bool a = GetArgBool(raw, field, false), b = GetArgBool(raw, field, true);
            if (a == b) { o.flags.set(mask, a ? TRUE : FALSE); changed = true; }
        };
        flag("run", SXmsOrder::flRun);
        flag("hostile", SXmsOrder::flHostile);
        if (o.kind == SXmsOrder::okPatrol && !o.route.size())
        {
            out = "{\"ok\":false,\"error\":\"a patrol needs a route: a way path of this level ('route')\"}";
            return true;
        }
        if (changed)
        {
            sp->m_Order = o;
            Scene->UndoSave();
            if (sp->Selected()) LTools->UpdateProperties(FALSE);
            UI->RedrawScene();
        }
        xr_string section;
        o.Section(section);
        out = "{\"ok\":true,\"name\":\"";
        out += JsonStr(sp->GetName());
        out += "\",\"section\":\"";
        out += JsonStr(sp->m_SpawnData.m_Data->name());
        out += "\",\"order\":{\"kind\":\"";
        out += SXmsOrder::KindKey(o.kind) ? SXmsOrder::KindKey(o.kind) : "none";
        out += "\",\"route\":\"";		out += JsonStr(o.route.c_str());
        out += "\",\"look\":\"";		out += JsonStr(o.look.c_str());
        out += "\",\"target\":\"";		out += JsonStr(o.target.size() ? o.target.c_str() : "actor");
        char nums[160];
        sprintf_s(nums, "\",\"radius\":%g,\"distance\":%g,\"run\":%s,\"hostile\":%s},\"changed\":%s,\"custom_data\":\"",
            o.radius, o.distance, o.flags.test(SXmsOrder::flRun) ? "true" : "false",
            o.flags.test(SXmsOrder::flHostile) ? "true" : "false", changed ? "true" : "false");
        out += nums;
        out += JsonStr(section.c_str());
        out += "\"}";
        return true;
    }

    // The viewport's right pills: the transform tool, object snap and its options, the grid, scale and angle snaps.
    // Arguments that are left out keep their value; a request with a bad one changes nothing.
    if (0 == xr_strcmp(cmd, "viewport_controls"))
    {
        Gizmo* gizmo = LTools->GetGimzo();
        bool support_move = false, support_rotate = false, support_scale = false;
        if (ESceneToolBase* tool = Scene->GetTool(LTools->CurrentClassID()))
            if (auto* control = tool->FindControl(0, etaSelect))
            {
                support_move = control->IsSupportMove();
                support_rotate = control->IsSupportRotate();
                support_scale = control->IsSupportScale();
            }

        char tool_arg[16] = {};
        const bool has_tool = GrizzlyMCP::GetArg(raw, "tool", tool_arg, sizeof(tool_arg)) && tool_arg[0];
        ETAction action = LTools->GetAction();
        Gizmo::EType type = gizmo->GetType();
        if (has_tool)
        {
            action = etaSelect;
            type = Gizmo::EType::None;
            bool supported = true;
            if (0 == _stricmp(tool_arg, "place"))			action = etaAdd;
            else if (0 == _stricmp(tool_arg, "move"))		{ type = Gizmo::EType::Move; supported = support_move; }
            else if (0 == _stricmp(tool_arg, "rotate"))		{ type = Gizmo::EType::Rotate; supported = support_rotate; }
            else if (0 == _stricmp(tool_arg, "scale"))		{ type = Gizmo::EType::Scale; supported = support_scale; }
            else if (0 != _stricmp(tool_arg, "select"))
            {
                out = "{\"ok\":false,\"error\":\"tool must be select, place, move, rotate or scale\"}";
                return true;
            }
            if (!supported)
            {
                out = "{\"ok\":false,\"error\":\"the ";
                out += JsonStr(ToolClassTitle(LTools->CurrentClassID()));
                out += " tool cannot ";
                out += tool_arg;
                out += " objects\"}";
                return true;
            }
        }

        struct SSnap { LPCSTR name; LPCSTR toggle; Gizmo::EType type; float step; bool on; bool has_step; };
        SSnap snaps[] =
        {
            { "grid",	"grid_snap",	Gizmo::EType::Move },
            { "scale",	"scale_snap",	Gizmo::EType::Scale },
            { "angle",	"angle_snap",	Gizmo::EType::Rotate },
        };
        for (SSnap& snap : snaps)
        {
            snap.step = gizmo->GetStep(snap.type);
            snap.has_step = GetArgFloat(raw, snap.name, snap.step);
            if (snap.has_step && !(snap.step > 0.f))
            {
                out = "{\"ok\":false,\"error\":\"";
                out += snap.name;
                out += " must be a step above 0\"}";
                return true;
            }
            snap.on = GetArgBool(raw, snap.toggle, gizmo->IsStepEnable(snap.type));
        }

        if (has_tool)
        {
            LTools->SetAction(action);
            gizmo->SetType(type);
        }
        for (const SSnap& snap : snaps)
        {
            if (snap.has_step)
                gizmo->SetStep(snap.type, snap.step);
            if (snap.on != gizmo->IsStepEnable(snap.type))
                gizmo->SwitchStep(snap.type, snap.on);
        }
        struct SSetting { LPCSTR name; u32 mask; };
        const SSetting settings[] = { { "object_snap", etfOSnap }, { "vertex_snap", etfVSnap }, { "normal_align", etfNormalAlign } };
        for (const SSetting& setting : settings)
        {
            const bool current = !!Tools->GetSettings(setting.mask);
            const bool wanted = GetArgBool(raw, setting.name, current);
            if (wanted != current)
                ExecCommand(COMMAND_SET_SETTINGS, setting.mask, wanted);
        }
        UI->RedrawScene();

        action = LTools->GetAction();
        type = gizmo->GetType();
        LPCSTR tool_name = action == etaAdd ? "place" : type == Gizmo::EType::Move ? "move"
            : type == Gizmo::EType::Rotate ? "rotate" : type == Gizmo::EType::Scale ? "scale" : "select";
        out = "{\"ok\":true,\"tool\":\"";
        out += tool_name;
        out += "\"";
        char tmp[256];
        for (const SSnap& snap : snaps)
        {
            sprintf_s(tmp, ",\"%s\":{\"on\":%s,\"step\":%g}", snap.name, gizmo->IsStepEnable(snap.type) ? "true" : "false",
                gizmo->GetStep(snap.type));
            out += tmp;
        }
        for (const SSetting& setting : settings)
        {
            sprintf_s(tmp, ",\"%s\":%s", setting.name, Tools->GetSettings(setting.mask) ? "true" : "false");
            out += tmp;
        }
        sprintf_s(tmp, ",\"supports\":{\"move\":%s,\"rotate\":%s,\"scale\":%s}}",
            support_move ? "true" : "false", support_rotate ? "true" : "false", support_scale ? "true" : "false");
        out += tmp;
        return true;
    }

    // The Tools panel's Snap Objects section: its two switches, its commands and what the list holds. Clear asks
    // nothing here: the caller has decided.
    if (0 == xr_strcmp(cmd, "snap_objects"))
    {
        UILeftBarForm* bar = MainForm ? MainForm->GetLeftBarForm() : nullptr;
        if (!bar)
        {
            out = "{\"ok\":false,\"error\":\"the Tools panel is not there\"}";
            return true;
        }
        char action[32] = {};
        u32 command = 0;
        if (GrizzlyMCP::GetArg(raw, "action", action, sizeof(action)) && action[0])
        {
            if (0 == _stricmp(action, "set_from_selection"))	command = COMMAND_SET_SNAP_OBJECTS;
            else if (0 == _stricmp(action, "add_selection"))	command = COMMAND_ADD_SEL_SNAP_OBJECTS;
            else if (0 == _stricmp(action, "remove_selection"))	command = COMMAND_DEL_SEL_SNAP_OBJECTS;
            else if (0 == _stricmp(action, "select"))			command = COMMAND_SELECT_SNAP_OBJECTS;
            else if (0 == _stricmp(action, "clear"))			command = COMMAND_CLEAR_SNAP_OBJECTS;
            else
            {
                out = "{\"ok\":false,\"error\":\"action must be set_from_selection, add_selection, remove_selection, select or clear\"}";
                return true;
            }
        }
        bar->SetUseSnapList(GetArgBool(raw, "use", bar->IsUseSnapList()));
        bar->SetSnapListMode(GetArgBool(raw, "pick", bar->IsSnapListMode()));
        if (command)
            ExecCommand(command);

        out = "{\"ok\":true,\"use\":";
        out += bar->IsUseSnapList() ? "true" : "false";
        out += ",\"pick\":";
        out += bar->IsSnapListMode() ? "true" : "false";
        out += ",\"objects\":[";
        if (ObjectList* lst = Scene->GetSnapList(true))
        {
            bool first = true;
            for (CCustomObject* obj : *lst)
            {
                out += first ? "\"" : ",\"";
                first = false;
                out += JsonStr(obj->GetName());
                out += "\"";
            }
        }
        out += "]}";
        return true;
    }

    // View > Game Lighting: the hour and weather of the linked game, the lights it would draw (GameLightingScene.cpp)
    if (0 == xr_strcmp(cmd, "game_lighting"))
    {
        GameLighting_Mcp(raw, out);
        return true;
    }

    if (0 == xr_strcmp(cmd, "view_mode"))
    {
        // presets first, so explicit fields can still override them
        char preset[64] = {};
        if (GrizzlyMCP::GetArg(raw, "preset", preset, sizeof(preset)) && preset[0])
        {
            if (0 == _stricmp(preset, "lit"))
            {
                EDevice->dwFillMode = D3DFILL_SOLID;
                psDeviceFlags.set(rsLighting, TRUE);
                psDeviceFlags.set(rsRenderTextures, TRUE);
            }
            else if (0 == _stricmp(preset, "unlit"))
            {
                EDevice->dwFillMode = D3DFILL_SOLID;
                psDeviceFlags.set(rsLighting, FALSE);
                psDeviceFlags.set(rsRenderTextures, TRUE);
            }
            else if (0 == _stricmp(preset, "game"))
            {
                // the game's lighting at the hour and weather game_lighting set (a new game's first night by default)
                EDevice->dwFillMode = D3DFILL_SOLID;
                psDeviceFlags.set(rsRenderTextures, TRUE);
                GGameLighting.Enable();
            }
            else if (0 == _stricmp(preset, "wireframe"))
            {
                EDevice->dwFillMode = D3DFILL_WIREFRAME;
            }
            else if (0 == _stricmp(preset, "point"))
            {
                EDevice->dwFillMode = D3DFILL_POINT;
            }
            else
            {
                out = "{\"ok\":false,\"error\":\"unknown preset\",\"presets\":"
                      "[\"unlit\",\"lit\",\"game\",\"wireframe\",\"point\"]}";
                return true;
            }
        }

        char fill[64] = {};
        if (GrizzlyMCP::GetArg(raw, "fill", fill, sizeof(fill)) && fill[0])
        {
            if      (0 == _stricmp(fill, "solid"))     EDevice->dwFillMode = D3DFILL_SOLID;
            else if (0 == _stricmp(fill, "wireframe")) EDevice->dwFillMode = D3DFILL_WIREFRAME;
            else if (0 == _stricmp(fill, "point"))     EDevice->dwFillMode = D3DFILL_POINT;
            else
            {
                out = "{\"ok\":false,\"error\":\"fill must be solid, wireframe or point\"}";
                return true;
            }
        }

        char shade[64] = {};
        if (GrizzlyMCP::GetArg(raw, "shade", shade, sizeof(shade)) && shade[0])
        {
            if      (0 == _stricmp(shade, "flat"))    EDevice->dwShadeMode = D3DSHADE_FLAT;
            else if (0 == _stricmp(shade, "gouraud")) EDevice->dwShadeMode = D3DSHADE_GOURAUD;
            else
            {
                out = "{\"ok\":false,\"error\":\"shade must be flat or gouraud\"}";
                return true;
            }
        }

        // every render flag the editor's Options menu exposes
        struct SFlagArg { pcstr name; u32 mask; };
        static const SFlagArg kFlags[] =
        {
            { "lighting",		rsLighting },
            { "textures",		rsRenderTextures },
            { "edged_faces",	rsEdgedFaces },
            { "filter_linear",	rsFilterLinear },
            { "fog",			rsFog },
            { "environment",	rsEnvironment },
            { "grid",			rsDrawGrid },
            { "safe_rect",		rsDrawSafeRect },
            { "stats",			rsStatistic },
        };
        for (const SFlagArg& f : kFlags)
        {
            const bool current = !!psDeviceFlags.test(f.mask);
            const bool wanted = GetArgBool(raw, f.name, current);
            if (wanted != current)
                psDeviceFlags.set(f.mask, wanted ? TRUE : FALSE);
        }
        g_SpawnHelpers = GetArgBool(raw, "helpers", g_SpawnHelpers);
        {
            const bool game = GetArgBool(raw, "game_lighting", GGameLighting.enabled);
            if (game != GGameLighting.enabled)
            {
                if (game)
                    GGameLighting.Enable();
                else
                    GGameLighting.Disable();
            }
        }

        UI->RedrawScene(true);

        pcstr fill_name = EDevice->dwFillMode == D3DFILL_WIREFRAME ? "wireframe"
                        : EDevice->dwFillMode == D3DFILL_POINT     ? "point" : "solid";
        pcstr shade_name = EDevice->dwShadeMode == D3DSHADE_FLAT ? "flat" : "gouraud";
        char tmp[1024];
        sprintf_s(tmp, sizeof(tmp),
            "{\"ok\":true,\"fill\":\"%s\",\"shade\":\"%s\","
            "\"lighting\":%s,\"textures\":%s,\"edged_faces\":%s,\"filter_linear\":%s,"
            "\"fog\":%s,\"environment\":%s,\"grid\":%s,\"safe_rect\":%s,\"stats\":%s,\"helpers\":%s,"
            "\"game_lighting\":%s,\"presets\":[\"unlit\",\"lit\",\"game\",\"wireframe\",\"point\"]}",
            fill_name, shade_name,
            psDeviceFlags.test(rsLighting) ? "true" : "false",
            psDeviceFlags.test(rsRenderTextures) ? "true" : "false",
            psDeviceFlags.test(rsEdgedFaces) ? "true" : "false",
            psDeviceFlags.test(rsFilterLinear) ? "true" : "false",
            psDeviceFlags.test(rsFog) ? "true" : "false",
            psDeviceFlags.test(rsEnvironment) ? "true" : "false",
            psDeviceFlags.test(rsDrawGrid) ? "true" : "false",
            psDeviceFlags.test(rsDrawSafeRect) ? "true" : "false",
            psDeviceFlags.test(rsStatistic) ? "true" : "false",
            g_SpawnHelpers ? "true" : "false",
            GGameLighting.enabled ? "true" : "false");
        out = tmp;
        return true;
    }

    if (0 == xr_strcmp(cmd, "scan_walls"))
    {
        // A horizontal lidar slice of the static geometry: rays fan out at one height from a grid of points, every
        // hit is a wall, a column or stock furniture there. Answers the hit cells of a res-metre grid over the area.
        float x0 = 0.f, x1 = 0.f, z0 = 0.f, z1 = 0.f, y = 0.f, step = 0.5f, reach = 6.f, res = 0.1f;
        if (!GetArgFloat(raw, "x0", x0) || !GetArgFloat(raw, "x1", x1) || !GetArgFloat(raw, "z0", z0) ||
            !GetArgFloat(raw, "z1", z1) || !GetArgFloat(raw, "y", y))
        {
            out = "{\"ok\":false,\"error\":\"x0, x1, z0, z1 and y are required\"}";
            return true;
        }
        GetArgFloat(raw, "step", step);
        GetArgFloat(raw, "reach", reach);
        GetArgFloat(raw, "res", res);
        step = _max(step, 0.1f);
        res = _max(res, 0.02f);
        reach = _max(reach, 0.5f);
        const int rays = _max(4, GetArgInt(raw, "rays", 24));
        if (x1 <= x0 || z1 <= z0 || (x1 - x0) > 80.f || (z1 - z0) > 80.f)
        {
            out = "{\"ok\":false,\"error\":\"the area must be non-empty and at most 80 x 80 m\"}";
            return true;
        }
        const u32 w = iCeil((x1 - x0) / res), h = iCeil((z1 - z0) / res);
        xr_vector<u32> cells;
        u32 cast = 0;
        g_RayPickAll = true;
        for (float ox = x0; ox <= x1; ox += step)
            for (float oz = z0; oz <= z1; oz += step)
                for (int i = 0; i < rays; ++i)
                {
                    const float a = PI_MUL_2 * float(i) / float(rays);
                    Fvector start, dir;
                    start.set(ox, y, oz);
                    dir.set(_sin(a), 0.f, _cos(a));
                    SRayPickInfo pinf;
                    pinf.inf.range = reach;
                    ++cast;
                    if (!Scene->RayPickObject(pinf.inf.range, start, dir, OBJCLASS_SCENEOBJECT, &pinf, 0))
                        continue;
                    Fvector hit;
                    RetrieveSceneObjPointAndNormal(hit, 0, pinf, 0);
                    if (!_valid(hit) || hit.x < x0 || hit.x >= x1 || hit.z < z0 || hit.z >= z1)
                        continue;
                    const u32 ix = _min(u32((hit.x - x0) / res), w - 1), iz = _min(u32((hit.z - z0) / res), h - 1);
                    cells.push_back(iz * w + ix);
                }
        g_RayPickAll = false;
        std::sort(cells.begin(), cells.end());
        cells.erase(std::unique(cells.begin(), cells.end()), cells.end());
        char head[256];
        sprintf_s(head, "{\"ok\":true,\"x0\":%.3f,\"z0\":%.3f,\"res\":%.3f,\"w\":%u,\"h\":%u,\"rays\":%u,\"cells\":[",
            x0, z0, res, w, h, cast);
        out = head;
        char num[16];
        for (size_t i = 0; i < cells.size(); ++i)
        {
            sprintf_s(num, i ? ",%u" : "%u", cells[i]);
            out += num;
        }
        out += "]}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "probe_vertical"))
    {
        // Straight up and straight down from a point through the static geometry: the ceiling over it and the
        // floor under it (where to hang a lamp, what a prop stands on). Answers the hit heights or null.
        float x = 0.f, y = 0.f, z = 0.f, reach = 30.f;
        if (!GetArgFloat(raw, "x", x) || !GetArgFloat(raw, "y", y) || !GetArgFloat(raw, "z", z))
        {
            out = "{\"ok\":false,\"error\":\"x, y and z are required\"}";
            return true;
        }
        GetArgFloat(raw, "reach", reach);
        auto cast = [&](float dir_y, float& hit_y) -> bool
        {
            Fvector start, dir;
            start.set(x, y, z);
            dir.set(0.f, dir_y, 0.f);
            SRayPickInfo pinf;
            pinf.inf.range = reach;
            if (!Scene->RayPickObject(pinf.inf.range, start, dir, OBJCLASS_SCENEOBJECT, &pinf, 0))
                return false;
            Fvector hit;
            RetrieveSceneObjPointAndNormal(hit, 0, pinf, 0);
            if (!_valid(hit))
                return false;
            hit_y = hit.y;
            return true;
        };
        g_RayPickAll = true;
        float up = 0.f, down = 0.f;
        const bool has_up = cast(1.f, up), has_down = cast(-1.f, down);
        g_RayPickAll = false;
        char tmp[192], a[32], b[32];
        sprintf_s(a, has_up ? "%.3f" : "null", up);
        sprintf_s(b, has_down ? "%.3f" : "null", down);
        sprintf_s(tmp, "{\"ok\":true,\"ceiling\":%s,\"floor\":%s}", a, b);
        out = tmp;
        return true;
    }

    if (0 == xr_strcmp(cmd, "drop_objects"))
    {
        if (Scene->locked())
        {
            out = "{\"ok\":false,\"error\":\"scene locked\"}";
            return true;
        }
        ObjectList batch;
        char names[4096] = {};
        xr_string missing;
        if (GrizzlyMCP::GetArg(raw, "names", names, sizeof(names)) && names[0])
        {
            for (char* cursor = names; cursor && *cursor;)
            {
                char* comma = strchr(cursor, ',');
                if (comma) *comma = 0;
                while (*cursor == ' ') ++cursor;
                char* end = cursor + xr_strlen(cursor);
                while (end > cursor && end[-1] == ' ') *--end = 0;
                if (*cursor)
                {
                    if (CCustomObject* obj = FindObjectRelaxed(cursor))
                        batch.push_back(obj);
                    else
                    {
                        if (!missing.empty()) missing += ",";
                        missing += cursor;
                    }
                }
                cursor = comma ? comma + 1 : 0;
            }
        }
        else
            CollectSelectedObjects(batch);

        if (batch.empty())
        {
            out = "{\"ok\":false,\"error\":\"nothing to drop (pass 'names' or select objects)\"}";
            return true;
        }

        char mode[32] = {};
        GrizzlyMCP::GetArg(raw, "mode", mode, sizeof(mode));
        const bool line_trace = (0 == _stricmp(mode, "line"));

		Scene->UndoTransformBegin(batch);
		const u32 moved = DropObjectsToGround(batch, GetArgBool(raw, "align_normal", false), line_trace);
        if (moved)
        {
			Scene->UndoTransformEnd();
            ExecCommand(COMMAND_UPDATE_PROPERTIES);
            UI->RedrawScene(true);
        }
		else
			Scene->UndoTransformCancel();
        char tmp[512];
        sprintf_s(tmp, sizeof(tmp), "{\"ok\":true,\"mode\":\"%s\",\"requested\":%u,\"dropped\":%u,\"missing\":\"%s\"}",
            line_trace ? "line" : "box", u32(batch.size()), moved, missing.c_str());
        out = tmp;
        return true;
    }

    if (0 == xr_strcmp(cmd, "rename_object"))
    {
        char name[256], new_name[256];
        if (!GrizzlyMCP::GetArg(raw, "name", name, sizeof(name)) ||
            !GrizzlyMCP::GetArg(raw, "new_name", new_name, sizeof(new_name)) || !new_name[0])
        {
            out = "{\"ok\":false,\"error\":\"need 'name' and non-empty 'new_name'\"}";
            return true;
        }
        CCustomObject* obj = FindObjectRelaxed(name);
        if (!obj)
        {
            out = "{\"ok\":false,\"error\":\"object not found\"}";
            return true;
        }
        strlwr(new_name);
        if (Scene->FindObjectByName(new_name, (CCustomObject*)0))
        {
            out = "{\"ok\":false,\"error\":\"new name already used\"}";
            return true;
        }
        obj->SetName(new_name);
        Scene->UndoSave();
        ExecCommand(COMMAND_UPDATE_PROPERTIES);
        RespondObjectJson(out, obj);
        return true;
    }

    if (0 == xr_strcmp(cmd, "delete_selected"))
    {
        if (Scene->locked())
        {
            out = "{\"ok\":false,\"error\":\"scene locked\"}";
            return true;
        }
        int removed = 0;
        for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
        {
            if (!it->second || it->first == OBJCLASS_DUMMY || !Scene->GetOTool(it->first)) continue;
            ObjectList& lst = Scene->ListObj(it->first);
            for (ObjectIt o = lst.begin(); o != lst.end(); ++o)
                if ((*o)->Selected()) ++removed;
        }
        Scene->RemoveSelection(OBJCLASS_DUMMY);
        Scene->UndoSave();
        // the Properties panel still holds the removed objects' fields: rebuild it now, or its
        // next draw reads freed memory (a token field of a deleted lamp took the editor down)
        LTools->UpdateProperties(TRUE);
        UI->RedrawScene(true);
        char tmp[64];
        sprintf_s(tmp, "{\"ok\":true,\"removed\":%d}", removed);
        out = tmp;
        return true;
    }

    if (0 == xr_strcmp(cmd, "save_scene"))
    {
        if (Scene->locked())
        {
            out = "{\"ok\":false,\"error\":\"scene locked\"}";
            return true;
        }
        char file[MAX_PATH] = {};
        GrizzlyMCP::GetArg(raw, "file", file, sizeof(file));
        xr_string fn = file[0] ? file : LTools->m_LastFileName.c_str();
        if (fn.empty())
        {
            out = "{\"ok\":false,\"error\":\"scene has no file yet - pass 'file' or Save As in the editor\"}";
            return true;
        }
        for (u32 i = 0; i < fn.size(); ++i) if (fn[i] == '/') fn[i] = '\\';
		xr_string save_error;
		if (!LevelMcpSaveScene(fn, save_error))
		{
			out = "{\"ok\":false,\"error\":\"";
			out += JsonStr(save_error.c_str());
			out += "\"}";
			return true;
		}
        out = "{\"ok\":true,\"file\":\"";
        for (u32 i = 0; i < fn.size(); ++i) out += (fn[i] == '\\') ? '/' : fn[i];
        out += "\"}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "undo") || 0 == xr_strcmp(cmd, "redo"))
    {
        if (Scene->locked())
        {
            out = "{\"ok\":false,\"error\":\"scene locked\"}";
            return true;
        }
        const bool is_undo = (0 == xr_strcmp(cmd, "undo"));
		if (LTools->GetGimzo()->IsFixed())
			LTools->GetGimzo()->Clear();
        const bool done = is_undo ? !!Scene->Undo() : !!Scene->Redo();
        if (!done)
        {
            if (Scene->UndoRollbackFailed())
                out = is_undo ? "{\"ok\":false,\"error\":\"undo rollback incomplete; scene marked modified\"}"
                              : "{\"ok\":false,\"error\":\"redo rollback incomplete; scene marked modified\"}";
            else
                out = is_undo ? "{\"ok\":false,\"error\":\"undo buffer empty\"}"
                              : "{\"ok\":false,\"error\":\"redo buffer empty\"}";
            return true;
        }
        LTools->Reset();
        ExecCommand(COMMAND_CHANGE_ACTION, etaSelect);
        UI->RedrawScene(true);
        out = "{\"ok\":true}";
        return true;
    }

    return false;
}
}
