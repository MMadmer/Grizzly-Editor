#pragma once

// What a single item SECTION can actually carry, so a spawn row shows the fields that mean
// something for it and nothing else: an epinephrine has no magazine and no wear, a rifle has
// no doses. One implementation for the inspector and the validator - a field the UI offers
// and a field the validator accepts have to be the same field.
//
// The rules come from the linked game's own configs, not from an assumption about X-Ray:
//   condition - use_condition when the section states it, otherwise the item family
//   magazine  - a magazined weapon class WITH ammo_mag_size > 0 (a knife has the key and no
//               magazine; ~227 binocular animation props have the key and are not weapons)
//   cartridge - a magazine plus an ammo_class naming more than one round
//   ammo box  - the ammo classes the factory accepts (box_size alone is NOT the test: the
//               cooking fuels carry it, and create_ammo throws on them)
//   doses     - an eatable class; max_uses defaults to 1 when the key is absent, so the
//               ceiling and "is it multi-dose" are different questions
//   upgrades  - a non-empty `upgrades` root (a bare `upgrades =` cancels an inherited one)
//
// Section inheritance needs no walking here: CInifile flattens `[a]:b,c` at load, last
// parent wins, so line_exist already answers for the whole chain.
//
// When there is no linked game, or the section is unknown, or its class is not in the family
// table, EVERY capability is reported present and every ceiling is 0 ("no ceiling"). The
// editor must never hide a field it cannot judge: a module ships sections the linked game's
// system.ltx has never heard of.

#include "NqAsset.h"

namespace NqItemCaps
{
	enum ECap
	{
		capCondition	= 1 << 0,
		capAmmo			= 1 << 1,	// a loaded magazine
		capAmmoKind		= 1 << 2,	// which cartridge is in it
		capUses			= 1 << 3,
		capUpgrades		= 1 << 4,
		capAmmoBox		= 1 << 5,	// the section IS ammo: `ammo` means the box content
		capAll			= 0x3F,
	};

	struct ECORE_API SCaps
	{
		u32						mask;		// capAll when `known` is false
		bool					known;		// false = no game, unknown section, or unknown class
		int						mag_size;	// ammo_mag_size, 0 when there is none
		int						box_size;	// box_size of an ammo section, 0 otherwise
		int						max_uses;	// effective max_uses, 0 when the section has none
		xr_vector<xr_string>	ammo_class;	// the rounds this weapon takes, in catalog order

		SCaps() : mask(capAll), known(false), mag_size(0), box_size(0), max_uses(0) {}
		bool Has(ECap c) const { return 0 != (mask & (u32)c); }
	};

	// Cached per section for as long as the linked game does not change.
	ECORE_API const SCaps&	Of			(LPCSTR section);
	// why a capability is absent, for the warning text ("it takes no magazine")
	ECORE_API LPCSTR		Why			(ECap c);
	// the field name as the author sees it ("ammo", "uses")
	ECORE_API LPCSTR		FieldName	(ECap c);
	// the row key -> capability it needs; capAll for a key that always applies
	ECORE_API u32			CapOfKey	(LPCSTR row_key);
	ECORE_API void			Invalidate	();
}
