#include "stdafx.h"
#include "LevelMcp.h"
#include "..\..\..\XrECore\Editor\EditorPda.h"
#include "..\..\..\XrECore\Editor\EditorGameContent.h"
#include "..\..\..\XrECore\Editor\UIEditLightAnim.h"
#include "..\..\..\XrECore\Editor\Nq\NqUtil.h"

// Previews over MCP: the PDA interface, the model and the texture tabs, and the three library tabs.
namespace LevelMcp
{
namespace
{
// -1 when the argument is not there, else 0 or 1
int ArgFlag(LPCSTR raw, LPCSTR field)
{
	xr_string value;
	if (!NqUtil::ArgString(raw, field, value) || value.empty())
		return NqUtil::ArgBool(raw, field, false) ? 1 : NqUtil::ArgBool(raw, field, true) ? -1 : 0;
	return value == "true" || value == "1" || value == "on" ? 1 : 0;
}

// A preview's answer holds what a frame lays out: its panes (pt), the scale a fit comes to. A call that changed its tab,
// or found it not drawn lately, owes the frames after it: the answer is "pending", the MCP pump holds it for them and
// runs the request again with answer_only set, which applies nothing (docs/UI_DESIGN.md, section 7).
void AppendPending(xr_string& out, bool owed)
{
	out += owed ? ",\"pending\":true" : ",\"pending\":false";
}

// texture_preview: open | state | close and the view (zoom fit | <factor>, mip, channel rgba | r | g | b | a, details).
// False with the error in `out`; `asked` = something the tab draws changed.
bool ApplyTexture(LPCSTR raw, const xr_string& action, bool& asked, xr_string& out)
{
	xr_string path, source, zoom, channel, err;
	NqUtil::ArgString(raw, "path", path);
	NqUtil::ArgString(raw, "source", source);
	if (action == "open")
	{
		const int src = source.empty() || source == "project" ? 0 : source == "editor" ? 1 : source == "darf" ? 2 : -1;
		if (path.empty() || src < 0)
		{
			NqUtil::JsonError(out, "open takes path= and source=project|editor|darf");
			return false;
		}
		for (char& c : path)
			if (c == '/')
				c = '\\';
		if (!UIImagePreview::Show(path.c_str(), src, &err))
		{
			NqUtil::JsonError(out, err);
			return false;
		}
		asked = true;
	}
	else if (action == "close")
	{
		if (XUI::Shell::IsOpen("texture"))
			XUI::Shell::Close("texture", XUI::Shell::Discard, nullptr);
		return true;
	}
	else if (action != "state")
	{
		NqUtil::JsonError(out, "action must be open, state or close");
		return false;
	}
	if (!UIImagePreview::IsOpen())
		return true;
	float factor = -1.f;
	if (NqUtil::ArgString(raw, "zoom", zoom) && !zoom.empty())
		factor = zoom == "fit" ? 0.f : float(atof(zoom.c_str()));
	NqUtil::ArgString(raw, "channel", channel);
	// rgb is the name the first version of this command used for the colour view
	const int ch = channel.empty() ? -1 : channel == "rgba" || channel == "rgb" ? 0 : channel == "r" ? 1 : channel == "g" ? 2 :
		channel == "b" ? 3 : channel == "a" ? 4 : -2;
	if (ch == -2 || (!zoom.empty() && zoom != "fit" && factor <= 0.f))
	{
		NqUtil::JsonError(out, "zoom is fit or a factor, channel rgba, r, g, b or a");
		return false;
	}
	const int mip = NqUtil::ArgInt(raw, "mip", -1), details = ArgFlag(raw, "details");
	if (!UIImagePreview::McpView(factor, mip, ch, details, err))
	{
		NqUtil::JsonError(out, err);
		return false;
	}
	asked = asked || factor >= 0.f || ch >= 0 || mip >= 0 || details >= 0;
	return true;
}
}

bool HandlePreview(LPCSTR cmd, LPCSTR raw, xr_string& out)
{
    // the module's own PDA interface (.pdaasset): text for an agent, a clickable preview tab
    if (0 == xr_strcmp(cmd, "pda_list"))			{ EditorPda::McpList(raw, out);			return true; }
    if (0 == xr_strcmp(cmd, "pda_get"))				{ EditorPda::McpGet(raw, out);			return true; }
    if (0 == xr_strcmp(cmd, "pda_write"))			{ EditorPda::McpWrite(raw, out);		return true; }
    if (0 == xr_strcmp(cmd, "pda_delete"))			{ EditorPda::McpDelete(raw, out);		return true; }
    if (0 == xr_strcmp(cmd, "pda_use"))				{ EditorPda::McpUse(raw, out);			return true; }
    if (0 == xr_strcmp(cmd, "pda_validate"))		{ EditorPda::McpValidate(raw, out);		return true; }
    if (0 == xr_strcmp(cmd, "pda_preview"))			{ EditorPda::McpPreview(raw, out);		return true; }
    // the library tabs: light animations, texture and sound descriptions
    if (0 == xr_strcmp(cmd, "library"))				{ EditorLibraryMcp(raw, out);			return true; }
    // opens the model preview tab - the same thing a double click in the content browser does, reachable
    // without driving the mouse; lighting=, wireframe=, bones=, details=, frame=, reset= set its view
    if (0 == xr_strcmp(cmd, "preview_model"))
    {
        const bool answer_only = NqUtil::ArgBool(raw, "answer_only", false);
        bool asked = false;
        if (!answer_only)
        {
            char name[512] = {}, source[64] = {};
            XFinedMCP::GetArg(raw, "name", name, sizeof(name));
            XFinedMCP::GetArg(raw, "source", source, sizeof(source));
            for (char* p = name; *p; ++p) if (*p == '/') *p = '\\';
            {
                char* w = name;
                for (const char* r = name; *r; ++r)
                    if (!(*r == '\\' && w > name && w[-1] == '\\')) *w++ = *r;
                *w = 0;
            }
            xr_string lighting, err;
            NqUtil::ArgString(raw, "lighting", lighting);
            if (!lighting.empty() && lighting != "studio" && lighting != "unlit")
            {
                NqUtil::JsonError(out, "lighting must be studio or unlit");
                return true;
            }

            if (0 == _stricmp(source, "darf"))
            {
                xr_string mount_err;
                if (!EditorGameContent::EnsureMounted(mount_err))
                {
                    NqUtil::JsonError(out, mount_err);
                    return true;
                }
                const int idx = EditorGameContent::Find(name);
                u32 sz = 0;
                u8* bytes = (idx >= 0) ? EditorGameContent::ReadBytes(idx, sz) : 0;
                if (!bytes || !sz)
                {
                    EditorGameContent::FreeBytes(bytes);
                    NqUtil::JsonError(out, "not found in the linked game install");
                    return true;
                }
                UIVisualPreview::ShowFromMemory(name, bytes, sz);
                EditorGameContent::FreeBytes(bytes);
            }
            else
                UIVisualPreview::Show(name[0] ? name : 0);

            const int wireframe = ArgFlag(raw, "wireframe"), bones = ArgFlag(raw, "bones"), details = ArgFlag(raw, "details");
            const bool frame = NqUtil::ArgBool(raw, "frame", false), reset = NqUtil::ArgBool(raw, "reset", false);
            if (!UIVisualPreview::McpView(lighting.empty() ? -1 : lighting == "unlit" ? 1 : 0, wireframe, bones, details, frame, reset, err))
            {
                NqUtil::JsonError(out, err);
                return true;
            }
            asked = name[0] || !lighting.empty() || wireframe >= 0 || bones >= 0 || details >= 0 || frame || reset;
        }
        // the preview is the "model" tab, up at once
        const bool active = XUI::Shell::IsActive("model");
        out = "{\"ok\":true,\"tab\":\"model\",\"active\":";
        out += active ? "true," : "false,";
        UIVisualPreview::McpState(out);
        AppendPending(out, !answer_only && active && (asked || !UIVisualPreview::DrawnLately()));
        out += "}";
        return true;
    }

	// the texture tab - what a double click on an image in the Content Browser opens - and its view
	if (0 == xr_strcmp(cmd, "texture_preview"))
	{
		xr_string action;
		NqUtil::ArgString(raw, "action", action);
		if (action.empty())
			action = "state";
		const bool answer_only = NqUtil::ArgBool(raw, "answer_only", false);
		bool asked = false;
		if (!answer_only && !ApplyTexture(raw, action, asked, out))
			return true;
		const bool active = XUI::Shell::IsActive("texture");
		out = "{\"ok\":true,\"tab\":\"texture\",\"active\":";
		out += active ? "true," : "false,";
		UIImagePreview::McpState(out);
		AppendPending(out, !answer_only && active && (asked || !UIImagePreview::DrawnLately()));
		out += "}";
		return true;
	}

    return false;
}
}
