#include "stdafx.h"
#include "NqItemCaps.h"
#include "NqUtil.h"
#include "NqPickers.h"
#include "../EditorGameContent.h"

namespace
{
	// ---- the item families -------------------------------------------------------
	// A section's `class` is a clsid STRING. In single player the C++ factory never sees
	// it - object_factory_register.cpp returns early off the dedicated-server check - so
	// the mapping lives in the game's own scripts\class_registrator.script:
	//     cs_register(factory, "<client class>", "<server class>", "<CLSID>", "<script>")
	// The server class is what says what an entity IS: se_ammo is ammo, se_weapon_magazined
	// carries a magazine, plain se_weapon (a knife) does not.
	enum EFamily
	{
		famUnknown = 0,
		famAmmo,
		famWeaponMag,		// a magazine, a cartridge choice
		famWeaponPlain,		// a knife: a weapon with no magazine
		famEatable,
		famWearable,		// outfit, helmet, backpack - condition only
		famExplosive,
		famArtefact,
		famPlain,
	};

	typedef xr_map<xr_string, EFamily> TFamilies;
	TFamilies	s_Fam;
	bool		s_FamReady = false;

	xr_string LowerOf(const xr_string& s)
	{
		xr_string r = s;
		for (u32 i = 0; i < r.size(); ++i) r[i] = (char)tolower((unsigned char)r[i]);
		return r;
	}

	EFamily FamilyOfServerClass(const xr_string& server_class_raw)
	{
		const xr_string s = LowerOf(server_class_raw);
		if (s.find("se_ammo") != xr_string::npos)		return famAmmo;
		// magazined, magazined_w_gl, shotgun, auto_shotgun - all carry a magazine
		if (s.find("magazined") != xr_string::npos ||
			s.find("shotgun") != xr_string::npos)		return famWeaponMag;
		if (s.find("se_weapon") != xr_string::npos)		return famWeaponPlain;
		if (s.find("se_eatable") != xr_string::npos)	return famEatable;
		if (s.find("se_outfit") != xr_string::npos ||
			s.find("se_helmet") != xr_string::npos ||
			s.find("se_backpack") != xr_string::npos)	return famWearable;
		if (s.find("se_explosive") != xr_string::npos ||
			s.find("se_grenade") != xr_string::npos)	return famExplosive;
		if (s.find("se_artefact") != xr_string::npos)	return famArtefact;
		return famPlain;
	}

	void AddFallback(LPCSTR clsid, EFamily f)
	{
		const xr_string k = clsid;
		if (s_Fam.find(k) == s_Fam.end()) s_Fam[k] = f;
	}

	// Engine clsids that the script never registers, so the script table alone would call
	// every medkit and every helmet "unknown" and hide their fields.
	void LoadFallbackFamilies()
	{
		AddFallback("AMMO",		famAmmo);
		AddFallback("AMMO_S",	famAmmo);
		AddFallback("S_VOG25",	famAmmo);
		AddFallback("S_OG7B",	famAmmo);
		AddFallback("S_M209",	famAmmo);
		AddFallback("II_FOOD",	famEatable);
		AddFallback("II_MEDKI",	famEatable);
		AddFallback("II_BANDG",	famEatable);
		AddFallback("II_ANTIR",	famEatable);
		AddFallback("II_BOTTL",	famEatable);
		AddFallback("II_EXPLO",	famExplosive);
		AddFallback("S_EXPLO",	famExplosive);
		AddFallback("G_F1_S",	famExplosive);
		AddFallback("G_RGD5_S",	famExplosive);
		// Dead Air ships helmets as E_HLMET and backpacks as artefacts; the clsid_game.h
		// names EQ_HLMET / EQ_BAKPK appear in no section at all
		AddFallback("E_HLMET",	famWearable);
		AddFallback("EQ_HLMET",	famWearable);
		AddFallback("EQU_STLK",	famWearable);
		AddFallback("EQU_SCIE",	famWearable);
		AddFallback("EQU_MLTR",	famWearable);
		AddFallback("EQU_EXO",	famWearable);
		AddFallback("E_STLK",	famWearable);
		AddFallback("SCRPTART",	famArtefact);
		AddFallback("ARTEFACT",	famArtefact);
	}

	// cs_register(factory, "client", "server", "CLSID", "script") - the CLSID is the 4th
	// string, the server class the 2nd. Commented-out registrations are everywhere in that
	// file (S_MEDKI, WP_MAGAZ...), and a line that starts with -- must not be read.
	void ParseClassRegistrator(const xr_string& text)
	{
		size_t at = 0;
		while (at < text.size())
		{
			size_t eol = text.find('\n', at);
			if (eol == xr_string::npos) eol = text.size();
			xr_string line = text.substr(at, eol - at);
			at = eol + 1;
			const xr_string trimmed = NqUtil::Trim(line);
			if (trimmed.size() >= 2 && trimmed[0] == '-' && trimmed[1] == '-') continue;
			const size_t call = trimmed.find("cs_register");
			if (call == xr_string::npos) continue;
			// collect the quoted strings of this call
			xr_vector<xr_string> q;
			size_t p = call;
			while (true)
			{
				const size_t a = trimmed.find('"', p);
				if (a == xr_string::npos) break;
				const size_t b = trimmed.find('"', a + 1);
				if (b == xr_string::npos) break;
				q.push_back(trimmed.substr(a + 1, b - a - 1));
				p = b + 1;
			}
			if (q.size() < 3) continue;
			const xr_string& server = q[1];
			const xr_string& clsid  = q[2];
			if (clsid.empty()) continue;
			s_Fam[clsid] = FamilyOfServerClass(server);
		}
	}

	void EnsureFamilies()
	{
		if (s_FamReady) return;
		s_FamReady = true;
		s_Fam.clear();
		xr_string err;
		if (EditorGameContent::EnsureMounted(err))
		{
			const int idx = EditorGameContent::Find("scripts\\class_registrator.script");
			if (idx >= 0)
			{
				u32 size = 0;
				if (u8* bytes = EditorGameContent::ReadBytes(idx, size))
				{
					xr_string text;
					text.assign((const char*)bytes, (const char*)bytes + size);
					EditorGameContent::FreeBytes(bytes);
					ParseClassRegistrator(text);
				}
			}
		}
		LoadFallbackFamilies();
	}

	// ---- section reading ----------------------------------------------------------
	// An empty value is ABSENT, not "present but blank": a bare `upgrades =` is the live
	// idiom for cancelling an inherited upgrade root, and the ini stores it as nothing.
	bool ReadStr(LPCSTR section, LPCSTR key, xr_string& out)
	{
		out.clear();
		if (!pSettings || !section || !section[0]) return false;
		if (!pSettings->section_exist(section)) return false;
		if (!pSettings->line_exist(section, key)) return false;
		LPCSTR v = pSettings->r_string(section, key);
		if (!v || !v[0]) return false;
		out = v;
		return true;
	}

	int ReadInt(LPCSTR section, LPCSTR key, int def)
	{
		xr_string s;
		if (!ReadStr(section, key, s)) return def;
		return atoi(s.c_str());
	}

	typedef xr_map<xr_string, NqItemCaps::SCaps> TCache;
	TCache					s_Cache;
	NqItemCaps::SCaps		s_Unknown;		// default-constructed: capAll, known = false
}

void NqItemCaps::Invalidate()
{
	s_Cache.clear();
	s_Fam.clear();
	s_FamReady = false;
}

const NqItemCaps::SCaps& NqItemCaps::Of(LPCSTR section)
{
	if (!section || !section[0]) return s_Unknown;
	// no linked game: every index is empty and W060 is silent - the capability verdict
	// has to be silent with it
	if (!pSettings || !NqPickers::Available()) return s_Unknown;

	const xr_string key = section;
	TCache::iterator it = s_Cache.find(key);
	if (it != s_Cache.end()) return it->second;

	SCaps c;
	if (!pSettings->section_exist(section))
	{
		s_Cache[key] = c;			// unknown section: everything offered, nothing warned
		return s_Cache[key];
	}

	EnsureFamilies();
	xr_string cls;
	EFamily fam = famUnknown;
	if (ReadStr(section, "class", cls))
	{
		TFamilies::iterator f = s_Fam.find(cls);
		if (f != s_Fam.end()) fam = f->second;
	}

	// ceilings are read whatever the family says: they are facts of the section
	c.mag_size	= ReadInt(section, "ammo_mag_size", 0);
	c.box_size	= ReadInt(section, "box_size", 0);
	c.max_uses	= ReadInt(section, "max_uses", 0);
	xr_string ac;
	if (ReadStr(section, "ammo_class", ac)) NqUtil::Split(ac.c_str(), ',', c.ammo_class);
	for (u32 i = 0; i < c.ammo_class.size(); ++i) c.ammo_class[i] = NqUtil::Trim(c.ammo_class[i]);

	if (fam == famUnknown)
	{
		// a class the tables do not know - a module's own, most likely. Offer everything
		// and keep the ceilings that are readable.
		s_Cache[key] = c;
		return s_Cache[key];
	}

	c.known = true;
	c.mask = 0;

	// condition: what the section says, else the family
	xr_string uc;
	if (ReadStr(section, "use_condition", uc))
	{
		const xr_string l = LowerOf(uc);
		if (l == "true" || l == "on" || l == "1") c.mask |= capCondition;
	}
	else if (fam == famWeaponMag || fam == famWeaponPlain || fam == famWearable ||
			 fam == famExplosive || fam == famArtefact)
		c.mask |= capCondition;

	if (fam == famAmmo)
	{
		c.mask |= capAmmoBox | capAmmo;		// `ammo` on an ammo row is the box content
		c.mag_size = 0;
	}
	else if (fam == famWeaponMag && c.mag_size > 0)
	{
		c.mask |= capAmmo;
		// a cartridge choice needs something to choose between
		if (c.ammo_class.size() > 1) c.mask |= capAmmoKind;
	}

	if (fam == famEatable) c.mask |= capUses;
	if (fam != famEatable) c.max_uses = 0;

	xr_string up;
	if (ReadStr(section, "upgrades", up)) c.mask |= capUpgrades;

	s_Cache[key] = c;
	return s_Cache[key];
}

LPCSTR NqItemCaps::Why(ECap c)
{
	switch (c)
	{
	case capCondition:	return "its condition is never read";
	case capAmmo:		return "it takes no magazine";
	case capAmmoKind:	return "it has only one cartridge";
	case capUses:		return "it has no doses";
	case capUpgrades:	return "it has no upgrades";
	case capAmmoBox:	return "it is not an ammo box";
	default:			return "it does not apply";
	}
}

LPCSTR NqItemCaps::FieldName(ECap c)
{
	switch (c)
	{
	case capCondition:	return "condition";
	case capAmmo:		return "ammo";
	case capAmmoKind:	return "ammo_kind";
	case capUses:		return "uses";
	case capUpgrades:	return "upgrades";
	default:			return "field";
	}
}

u32 NqItemCaps::CapOfKey(LPCSTR row_key)
{
	if (!row_key) return capAll;
	const xr_string k = row_key;
	if (k == "condition")	return capCondition;
	if (k == "ammo")		return capAmmo;
	if (k == "ammo_kind")	return capAmmoKind;
	if (k == "uses")		return capUses;
	if (k == "upgrades")	return capUpgrades;
	return capAll;			// section, count, ref - always applicable
}
