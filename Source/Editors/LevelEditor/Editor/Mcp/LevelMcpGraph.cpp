#include "stdafx.h"
#include "LevelMcp.h"
#include "..\..\..\XrECore\Editor\Nq\NqMcp.h"
#include "..\..\UI\QuestGraph\UIQuestGraph.h"

// Quest and behaviour graphs over MCP: the headless NQ service and the graph tabs.
namespace LevelMcp
{
bool HandleGraph(LPCSTR cmd, LPCSTR raw, xr_string& out)
{
    if (!Scene) return false;

    // quest graph (NQ) commands: project-scoped, no scene needed, no dialogs
	if (0 == xr_strcmp(cmd, "quest_close")) { UIQuestGraph::McpClose(raw, out); return true; }
    if (0 == xr_strcmp(cmd, "quest_view")) { UIQuestGraph::McpView(raw, out); return true; }
    if (0 == xr_strcmp(cmd, "quest_edit")) { UIQuestGraph::McpEdit(raw, out); return true; }
    if (0 == xr_strcmp(cmd, "quest_find")) { UIQuestGraph::McpFind(raw, out); return true; }
    if (0 == xr_strcmp(cmd, "quest_bookmarks")) { UIQuestGraph::McpBookmarks(raw, out); return true; }
    if (0 == xr_strcmp(cmd, "quest_history")) { UIQuestGraph::McpHistory(raw, out); return true; }
    if (0 == xr_strcmp(cmd, "quest_minimap")) { UIQuestGraph::McpMinimap(raw, out); return true; }
    if (NqMcp::Handle(cmd, raw, out)) return true;

    return false;
}
}
