#include "stdafx.h"
#pragma hdrstop

#include "../Public/PropertiesListTypes.h"
#include "../Public/ShortcutText.h"
//------------------------------------------------------------------------------

xr_string	ShortcutValue::GetDrawText		(TOnDrawTextEvent )
{
	return value->key ? ShortcutText(*value) : xr_string("None");
}

// the game types a spawn takes part in, by the names the game's menus give them
xr_string GameTypeValue::GetDrawText(TOnDrawTextEvent)
{
	static const struct { EGameIDs id; const char* name; } kTypes[] =
	{
		{ eGameIDSingle, "Single Player" },
		{ eGameIDDeathmatch, "Deathmatch" },
		{ eGameIDTeamDeathmatch, "Team Deathmatch" },
		{ eGameIDArtefactHunt, "Artefact Hunt" },
		{ eGameIDCaptureTheArtefact, "Capture the Artefact" },
	};
	xr_string text;
	for (const auto& type : kTypes)
		if (GetValue().m_GameType.test(u16(type.id)))
		{
			if (!text.empty())
				text += ", ";
			text += type.name;
		}
	return text.empty() ? xr_string("None") : text;
}
