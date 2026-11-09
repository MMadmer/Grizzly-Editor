#include "stdafx.h"
#include "LevelMcp.h"
#include "..\..\..\..\xrServerEntities\xrServer_Objects_ALife_All.h"
#include "..\EditorModScene.h"
#include "..\..\..\XrECore\Editor\Nq\NqUtil.h"
#include "..\..\..\XrEProps\UIPropsLocal.h"

namespace
{
// a row's value as property_inspector gives it: as drawn, but a row of playback controls names them the way property_set
// takes them ("First Frame,Play,...") instead of by the engine's glyphs
xr_string ValueText(PropItem* item)
{
	if (item->Type() == PROP_BUTTON)
		if (ButtonValue* V = dynamic_cast<ButtonValue*>(item->GetFrontValue()))
		{
			xr_string names;
			for (const shared_str& caption : V->value)
			{
				const char* name = nullptr;
				XUI_Local::TransportOf(caption.c_str(), &name);
				if (!names.empty())
					names += ",";
				names += name ? name : "";
			}
			return names;
		}
	return item->GetDrawText();
}
}

// Details and Level Settings over MCP: the property rows, the choosers behind them and the spawn fields they set.
namespace LevelMcp
{
bool HandleProps(LPCSTR cmd, LPCSTR raw, xr_string& out)
{
	if (0 == xr_strcmp(cmd, "property_inspector"))
	{
		char action[32] = {}, target[32] = {}, filter[256] = {};
		GrizzlyMCP::GetArg(raw, "action", action, sizeof(action));
		GrizzlyMCP::GetArg(raw, "target", target, sizeof(target));
		GrizzlyMCP::GetArg(raw, "filter", filter, sizeof(filter));
		if (!action[0])
			xr_strcpy(action, filter[0] ? "filter" : "get");
		if (!target[0])
			xr_strcpy(target, "selection");

		UIPropertiesForm* properties = nullptr;
		bool world_target = false;
		if (0 == _stricmp(target, "selection"))
		{
			properties = LTools->GetProperties();
		}
		else if (0 == _stricmp(target, "world"))
		{
			properties = LTools->GetWorldProperties();
			world_target = true;
		}
		else
		{
			out = "{\"ok\":false,\"error\":\"target must be selection or world\"}";
			return true;
		}

		if (0 == _stricmp(action, "filter"))
			properties->SetFilter(filter);
		else if (0 == _stricmp(action, "expand"))
			properties->ExpandAll();
		else if (0 == _stricmp(action, "collapse"))
			properties->CollapseAll();
		else if (0 == _stricmp(action, "open"))
		{
			// an opener: the Level tab comes up and the panel in front of its dock node
			XUI::Shell::Activate("level");
			if (world_target)
			{
				MainForm->GetWorldPropertiesFrom()->Open();
				XUI::Shell::FocusWindow("Level Settings###World Properties");
			}
			else
			{
				MainForm->GetPropertiesFrom()->Open();
				XUI::Shell::FocusWindow("Details###Properties");
			}
		}
		else if (0 == _stricmp(action, "close"))
		{
			if (world_target)
				MainForm->GetWorldPropertiesFrom()->Close();
			else
				MainForm->GetPropertiesFrom()->Close();
		}
		else if (0 != _stricmp(action, "get"))
		{
			out = "{\"ok\":false,\"error\":\"action must be get, filter, expand, collapse, open, or close\"}";
			return true;
		}

		const bool panel_open = world_target
			? !MainForm->GetWorldPropertiesFrom()->IsClosed()
			: !MainForm->GetPropertiesFrom()->IsClosed();
		char counts[128];
		sprintf_s(counts, "\",\"visible\":%u,\"total\":%u,\"open\":%s}",
			properties->VisiblePropertyCount(), properties->PropertyCount(), panel_open ? "true" : "false");
		out = "{\"ok\":true,\"target\":\"";
		out += target;
		out += "\",\"filter\":\"";
		out += JsonStr(properties->GetFilter());
		out += counts;
		// the rows themselves, in the grid's order: key, the label the grid shows, the value as drawn, whether it can be
		// edited, whether the selected objects differ on it, and its help - the tooltip and the help footer say the same
		out.pop_back();
		out += ",\"rows\":[";
		const int limit = GetArgInt(raw, "limit", 200);
		int rows = 0;
		PropItemVec ordered;
		properties->RowsInOrder(ordered);
		for (PropItem* item : ordered)
		{
			if (!item || rows >= limit)
				continue;
			const bool mixed = item->m_Flags.test(PropItem::flMixed) && !item->m_Flags.test(PropItem::flIgnoreMixed);
			out += rows++ ? ",{\"key\":\"" : "{\"key\":\"";
			out += JsonStr(item->Key());
			out += "\",\"label\":\"";
			out += JsonStr(UIPropertiesForm::LabelOf(item->Key()).c_str());
			out += "\",\"value\":\"";
			out += JsonStr(ValueText(item).c_str());
			out += item->Enabled() && !properties->IsReadOnly() ? "\",\"enabled\":true" : "\",\"enabled\":false";
			out += mixed ? ",\"mixed\":true,\"help\":\"" : ",\"mixed\":false,\"help\":\"";
			out += JsonStr(properties->GetHelp(item).c_str());
			// the unit a number is drawn in ("m", the degree sign); a value given to property_set is the number alone
			const xr_string unit = properties->GetUnit(item);
			if (!unit.empty())
			{
				out += "\",\"unit\":\"";
				out += JsonStr(unit.c_str());
			}
			out += "\"}";
		}
		out += "]}";
		return true;
	}

	// The editor's shortcuts, the Preferences tab's Keyboard page: list them, give one a new shortcut (the command that
	// had it lets it go, as in the tab) or take one away. Saved at once.
	//   {"cmd":"shortcuts"}   {"cmd":"shortcuts","action":"set","command":"COMMAND_UNDO","keys":"Ctrl+Z"}
	if (0 == xr_strcmp(cmd, "shortcuts"))
	{
		if (!EPrefs)
		{
			NqUtil::JsonError(out, "editor preferences are not ready");
			return true;
		}
		xr_string action, command, keys, filter;
		NqUtil::ArgString(raw, "action", action);
		NqUtil::ArgString(raw, "command", command);
		NqUtil::ArgString(raw, "keys", keys);
		NqUtil::ArgString(raw, "filter", filter);
		if (action.empty())
			action = "list";
		auto append_row = [](xr_string& json, SESubCommand* sub)
		{
			json += "{\"id\":";
			NqUtil::JsonString(json, CCustomPreferences::ShortcutId(sub));
			json += ",\"label\":";
			NqUtil::JsonString(json, CCustomPreferences::ShortcutLabel(sub));
			json += ",\"keys\":";
			NqUtil::JsonString(json, XUI::ShortcutLabel(sub->shortcut));
			json += "}";
		};
		if (action == "list")
		{
			out = "{\"ok\":true,\"shortcuts\":[";
			bool first = true;
			for (SECommand* c : GetEditorCommands())
				if (c && c->editable)
					for (SESubCommand* sub : c->sub_commands)
					{
						const xr_string text = CCustomPreferences::ShortcutId(sub) + " " + CCustomPreferences::ShortcutLabel(sub) + " " +
							XUI::ShortcutLabel(sub->shortcut);
						if (!XUI::Match(filter.c_str(), text.c_str()))
							continue;
						if (!first)
							out += ",";
						first = false;
						append_row(out, sub);
					}
			out += "]}";
			return true;
		}
		if (action != "set" && action != "clear")
		{
			NqUtil::JsonError(out, "action must be list, set or clear");
			return true;
		}
		// by its id (COMMAND_UNDO, COMMAND_ZOOM_EXTENTS."Selected") or by the label the Keyboard page shows
		SESubCommand* target = nullptr;
		for (SECommand* c : GetEditorCommands())
			if (c && c->editable)
				for (SESubCommand* sub : c->sub_commands)
					if (!target && (!_stricmp(CCustomPreferences::ShortcutId(sub).c_str(), command.c_str()) ||
						!_stricmp(CCustomPreferences::ShortcutLabel(sub).c_str(), command.c_str())))
						target = sub;
		if (!target)
		{
			NqUtil::JsonError(out, "no command '" + command + "' takes a shortcut (shortcuts action=list names them)");
			return true;
		}
		xr_shortcut shortcut;
		if (action == "set" && !UIKeyPressForm::Parse(keys.c_str(), shortcut))
		{
			NqUtil::JsonError(out, "keys must be a shortcut such as Ctrl+Shift+S, F5 or Num Del");
			return true;
		}
		SESubCommand* previous = action == "set" ? CCustomPreferences::ShortcutOwner(shortcut, target) : nullptr;
		if (action == "set")
			EPrefs->AssignShortcut(target, shortcut);
		else
		{
			target->shortcut = xr_shortcut();
			EPrefs->Save();
		}
		out = "{\"ok\":true,\"shortcut\":";
		append_row(out, target);
		out += ",\"moved_from\":";
		if (previous)
			NqUtil::JsonString(out, CCustomPreferences::ShortcutId(previous));
		else
			out += "null";
		out += "}";
		return true;
	}

    if (!Scene) return false;

	// A row of Details (target=selection) or Level Settings (target=world) given a value as text, applied the way
	// editing the row applies it: its checks run, every selected object takes it, one undo step.
	//   {"cmd":"property_set","key":"Transform\\Position","value":"1, 2, 3"}
	// key: the row's whole key, or its last part when that names one row. value: a number, true/false, x, y, z,
	// #RRGGBB, a name from the row's list, text, a shortcut, or a button's caption to press it.
	if (0 == xr_strcmp(cmd, "property_set"))
	{
		xr_string target, key, value;
		NqUtil::ArgString(raw, "target", target);
		NqUtil::ArgString(raw, "key", key);
		const bool has_value = NqUtil::ArgString(raw, "value", value);
		if (target.empty())
			target = "selection";
		if (target != "selection" && target != "world")
		{
			NqUtil::JsonError(out, "target must be selection or world");
			return true;
		}
		if (key.empty() || !has_value)
		{
			NqUtil::JsonError(out, "property_set takes key= and value=");
			return true;
		}
		UIPropertiesForm* properties = target == "world" ? LTools->GetWorldProperties() : LTools->GetProperties();
		PropItem* item = properties->FindItem(key.c_str());
		if (!item)
			item = properties->FindItemOfName(key.c_str());
		if (!item)
		{
			NqUtil::JsonError(out, "no row '" + key + "' (property_inspector target=" + target + " lists them)");
			return true;
		}
		xr_string err;
		if (!properties->SetValueText(item, value.c_str(), err))
		{
			NqUtil::JsonError(out, xr_string(item->Key()) + " " + err);
			return true;
		}
		Scene->UndoSave();
		ExecCommand(COMMAND_UPDATE_PROPERTIES);
		UI->RedrawScene();
		out = "{\"ok\":true,\"target\":";
		NqUtil::JsonString(out, target);
		out += ",\"key\":";
		NqUtil::JsonString(out, item->Key());
		out += ",\"value\":";
		NqUtil::JsonString(out, ValueText(item));
		out += "}";
		return true;
	}

    // The MCP half of a PROP_CHOOSE row: open the modal the row owns, pick one item
    // by its tree path (a leaf carries a trailing '*') and confirm. Same entry point
    // the mouse uses, so a chooser that works here works for the author too.
    //   {"cmd":"property_choose","action":"open","key":"Visual"}
    //   {"cmd":"property_choose","action":"pick","item":"dynamics\\box\\box_metall_01*"}
    //   {"cmd":"property_choose","action":"status","key":"Visual"}
    if (0 == xr_strcmp(cmd, "property_choose"))
    {
        char action[32] = {}, key[256] = {}, item[256] = {};
        GrizzlyMCP::GetArg(raw, "action", action, sizeof(action));
        GrizzlyMCP::GetArg(raw, "key", key, sizeof(key));
        GrizzlyMCP::GetArg(raw, "item", item, sizeof(item));
        UIPropertiesForm* props = LTools->GetProperties();
        if (0 == _stricmp(action, "open"))
        {
            PropItem* p = props->FindItemOfName(key[0] ? key : "Visual");
            if (!p) { out = "{\"ok\":false,\"error\":\"no such property row\"}"; return true; }
            const bool started = props->BeginChooseEdit(p);
            char tmp[512];
            sprintf_s(tmp, "{\"ok\":%s,\"key\":\"%s\",\"value\":\"%s\"}",
                started ? "true" : "false", JsonEscapePath(p->Key()).c_str(), JsonEscapePath(p->GetDrawText().c_str()).c_str());
            out = tmp;
            return true;
        }
        if (0 == _stricmp(action, "cancel"))
        {
            const bool closed = UIChooseForm::CancelForm();
            char tmp[64];
            sprintf_s(tmp, "{\"ok\":%s}", closed ? "true" : "false");
            out = tmp;
            return true;
        }
        if (0 == _stricmp(action, "select"))
        {
            const bool picked = UIChooseForm::Pick(item);
            char tmp[256];
            sprintf_s(tmp, "{\"ok\":%s}", picked ? "true" : "false");
            out = tmp;
            return true;
        }
        if (0 == _stricmp(action, "pick"))
        {
            const bool picked = UIChooseForm::PickAndConfirm(item);
            char tmp[256];
            sprintf_s(tmp, "{\"ok\":%s}", picked ? "true" : "false");
            out = tmp;
            return true;
        }
        PropItem* p = props->FindItemOfName(key[0] ? key : "Visual");
        char tmp[768];
        sprintf_s(tmp, "{\"ok\":true,\"form_alive\":%s,\"pending\":%s,\"selected\":\"%s\",\"value\":\"%s\"}",
            UIChooseForm::IsActive() ? "true" : "false",
            props->HasPendingChooseEdit() ? "true" : "false",
            JsonEscapePath(UIChooseForm::SelectedName()).c_str(),
            p ? JsonEscapePath(p->GetDrawText().c_str()).c_str() : "");
        out = tmp;
        return true;
    }

    // {"name":..., "visual":"dynamics\\box\\box_wood_01"} - sets Model\Visual of a spawn
    // entity, the MCP half of the property inspector chooser. Empty visual is
    // refused: clearing it re-reads the section default only on reload, which lies.
    if (0 == xr_strcmp(cmd, "spawn_set_visual"))
    {
        char name[256] = {}, visual[256] = {};
        GrizzlyMCP::GetArg(raw, "name", name, sizeof(name));
        GrizzlyMCP::GetArg(raw, "visual", visual, sizeof(visual));
        if (!name[0] || !visual[0])
        {
            out = "{\"ok\":false,\"error\":\"need 'name' and non-empty 'visual'\"}";
            return true;
        }
        CCustomObject* obj = FindObjectRelaxed(name);
        CSpawnPoint* sp = obj ? dynamic_cast<CSpawnPoint*>(obj) : 0;
        if (!sp)
        {
            out = "{\"ok\":false,\"error\":\"spawn object not found\"}";
            return true;
        }
        if (!sp->m_SpawnData.Valid())
        {
            out = "{\"ok\":false,\"error\":\"object carries no spawn data\"}";
            return true;
        }
        ISE_Visual* vis = sp->m_SpawnData.m_Data->visual();
        if (!vis)
        {
            out = "{\"ok\":false,\"error\":\"this section has no visual\"}";
            return true;
        }
        // normalized to what configs carry: meshes-relative, backslashes, no .ogf
        char vname[256];
        {
            LPCSTR src = visual;
            if (0 == _strnicmp(src, "meshes\\", 7) || 0 == _strnicmp(src, "meshes/", 7)) src += 7;
            strncpy_s(vname, src, _TRUNCATE);
            for (char* c = vname; *c; ++c) if (*c == '/') *c = '\\';
            if (char* dot = strrchr(vname, '.')) *dot = 0;
        }
        // a name nothing resolves would crash the game at online spawn - refuse it
        if (!EditorModScene::VisualReachable(vname))
        {
            out = "{\"ok\":false,\"error\":\"mesh not found in the game or the project (meshes\\\\<visual>.ogf)\"}";
            return true;
        }
        vis->set_visual(vname);
        // same signal the UI chooser sends: the viewport model reloads next frame
        sp->m_SpawnData.m_Data->set_editor_flag(ISE_Abstract::flVisualChange);
        Scene->UndoSave();
        ExecCommand(COMMAND_UPDATE_PROPERTIES);
        UI->RedrawScene(true);
        char tmp[400];
        sprintf_s(tmp, "{\"ok\":true,\"visual\":\"%s\",\"draws_nothing\":%s}",
            JsonEscapePath(vname).c_str(), EditorModScene::VisualDrawsNothing(vname) ? "true" : "false");
        out = tmp;
        return true;
    }

    // {"name":..., "bones":"link"} - Model\Fixed bones of a physic-object spawn
    // entity: the bones welded to the world, so the prop stands where it is put.
    // Empty bones clears the field (a free physics body again).
    if (0 == xr_strcmp(cmd, "spawn_set_fixed_bones"))
    {
        char name[256] = {}, bones[256] = {};
        GrizzlyMCP::GetArg(raw, "name", name, sizeof(name));
        GrizzlyMCP::GetArg(raw, "bones", bones, sizeof(bones));
        if (!name[0])
        {
            out = "{\"ok\":false,\"error\":\"need 'name'\"}";
            return true;
        }
        CCustomObject* obj = FindObjectRelaxed(name);
        CSpawnPoint* sp = obj ? dynamic_cast<CSpawnPoint*>(obj) : 0;
        if (!sp || !sp->m_SpawnData.Valid())
        {
            out = "{\"ok\":false,\"error\":\"spawn object not found\"}";
            return true;
        }
        CSE_ALifeObjectPhysic* po = dynamic_cast<CSE_ALifeObjectPhysic*>(sp->m_SpawnData.m_Data);
        if (!po)
        {
            out = "{\"ok\":false,\"error\":\"this section has no fixed bones (not a physic object)\"}";
            return true;
        }
        po->fixed_bones = bones;
        Scene->UndoSave();
        ExecCommand(COMMAND_UPDATE_PROPERTIES);
        char tmp[340];
        sprintf_s(tmp, "{\"ok\":true,\"fixed_bones\":\"%s\"}", JsonEscapePath(bones).c_str());
        out = tmp;
        return true;
    }

    return false;
}
}
