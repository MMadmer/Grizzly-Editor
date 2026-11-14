//! What a single item SECTION can carry, so a spawn row is offered and checked
//! for the fields that mean something for it: an epinephrine has no magazine and no wear, a
//! rifle has no doses.
//!
//! The rules come from the linked game's own configs: an item's `class` names a clsid, and the
//! game's `scripts\class_registrator.script` says which server class it is (`se_ammo` is ammo,
//! `se_weapon_magazined` carries a magazine). Without a linked game, for an unknown section or a
//! class the tables do not know, EVERY capability is present and no ceiling applies: a module
//! ships sections the linked game has never heard of.

use std::sync::Arc;

use gz_ltx::Ltx;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::text::{c_atoi, split, trim};

pub const CONDITION: u32 = 1;
/// A loaded magazine.
pub const AMMO: u32 = 2;
/// Which cartridge is in it.
pub const AMMO_KIND: u32 = 4;
pub const UPGRADES: u32 = 8;
/// The section IS ammo: `ammo` means the box content.
pub const AMMO_BOX: u32 = 16;
pub const ALL: u32 = 0x1F;

/// What one section can carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caps {
    /// [`ALL`] when `known` is false.
    pub mask: u32,
    /// False: no game, an unknown section, or an unknown class.
    pub known: bool,
    /// `ammo_mag_size`, 0 without one.
    pub mag_size: i32,
    /// `box_size` of an ammo section, 0 otherwise.
    pub box_size: i32,
    /// The rounds this weapon takes, in catalog order.
    pub ammo_class: Vec<String>,
    /// The upgrade ELEMENT ids this section can install.
    pub upgrades: Vec<String>,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            mask: ALL,
            known: false,
            mag_size: 0,
            box_size: 0,
            ammo_class: Vec::new(),
            upgrades: Vec::new(),
        }
    }
}

impl Caps {
    #[must_use]
    pub fn has(&self, cap: u32) -> bool {
        self.mask & cap != 0
    }

    /// What the engine gives an item of this section when the quest says nothing.
    #[must_use]
    pub fn full_ammo(&self) -> i32 {
        if self.has(AMMO_BOX) { self.box_size } else { self.mag_size }
    }
}

/// Why a capability is absent, for the warning text.
#[must_use]
pub fn why(cap: u32) -> &'static str {
    match cap {
        CONDITION => "its condition is never read",
        AMMO => "it takes no magazine",
        AMMO_KIND => "it has only one cartridge",
        UPGRADES => "it has no upgrades",
        AMMO_BOX => "it is not an ammo box",
        _ => "it does not apply",
    }
}

/// The field name as the author sees it.
#[must_use]
pub fn field_name(cap: u32) -> &'static str {
    match cap {
        CONDITION => "condition",
        AMMO => "ammo",
        AMMO_KIND => "ammo_kind",
        UPGRADES => "upgrades",
        _ => "field",
    }
}

/// The capability a row key needs; [`ALL`] for one that always applies.
#[must_use]
pub fn cap_of_key(key: &str) -> u32 {
    match key {
        "condition" => CONDITION,
        "ammo" => AMMO,
        "ammo_kind" => AMMO_KIND,
        "upgrades" => UPGRADES,
        _ => ALL,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Ammo,
    WeaponMag,
    WeaponPlain,
    Eatable,
    Wearable,
    Explosive,
    Artefact,
    Plain,
}

fn family_of_server_class(server: &str) -> Family {
    let s = server.to_ascii_lowercase();
    if s.contains("se_ammo") {
        Family::Ammo
    } else if s.contains("magazined") || s.contains("shotgun") {
        Family::WeaponMag
    } else if s.contains("se_weapon") {
        Family::WeaponPlain
    } else if s.contains("se_eatable") {
        Family::Eatable
    } else if s.contains("se_outfit") || s.contains("se_helmet") || s.contains("se_backpack") {
        Family::Wearable
    } else if s.contains("se_explosive") || s.contains("se_grenade") {
        Family::Explosive
    } else if s.contains("se_artefact") {
        Family::Artefact
    } else {
        Family::Plain
    }
}

/// `cs_register(factory, "client", "server", "CLSID", ...)` lines: the server class is the
/// second string, the clsid the third. A line that starts with `--` is not read.
fn parse_class_registrator(text: &str, out: &mut FxHashMap<String, Family>) {
    for line in text.split('\n') {
        let t = trim(line);
        if t.starts_with("--") {
            continue;
        }
        let Some(call) = t.find("cs_register") else { continue };
        let mut q: Vec<&str> = Vec::new();
        let mut p = call;
        while let Some(a) = t.get(p..).and_then(|r| r.find('"')).map(|i| p + i) {
            let Some(b) = t.get(a + 1..).and_then(|r| r.find('"')).map(|i| a + 1 + i) else { break };
            q.push(t.get(a + 1..b).unwrap_or(""));
            p = b + 1;
        }
        let (Some(server), Some(clsid)) = (q.get(1), q.get(2)) else { continue };
        if clsid.is_empty() {
            continue;
        }
        out.insert((*clsid).to_owned(), family_of_server_class(server));
    }
}

/// Engine clsids the script never registers.
const FALLBACK: &[(&str, Family)] = &[
    ("AMMO", Family::Ammo),
    ("AMMO_S", Family::Ammo),
    ("S_VOG25", Family::Ammo),
    ("S_OG7B", Family::Ammo),
    ("S_M209", Family::Ammo),
    ("II_FOOD", Family::Eatable),
    ("II_MEDKI", Family::Eatable),
    ("II_BANDG", Family::Eatable),
    ("II_ANTIR", Family::Eatable),
    ("II_BOTTL", Family::Eatable),
    ("II_EXPLO", Family::Explosive),
    ("S_EXPLO", Family::Explosive),
    ("G_F1_S", Family::Explosive),
    ("G_RGD5_S", Family::Explosive),
    ("E_HLMET", Family::Wearable),
    ("EQ_HLMET", Family::Wearable),
    ("EQU_STLK", Family::Wearable),
    ("EQU_SCIE", Family::Wearable),
    ("EQU_MLTR", Family::Wearable),
    ("EQU_EXO", Family::Wearable),
    ("E_STLK", Family::Wearable),
    ("SCRPTART", Family::Artefact),
    ("ARTEFACT", Family::Artefact),
];

/// The capabilities of the linked game's sections, computed per section on first use.
pub struct ItemCaps {
    ltx: Arc<Ltx>,
    families: FxHashMap<String, Family>,
    cache: Mutex<FxHashMap<String, Arc<Caps>>>,
}

impl ItemCaps {
    /// From the game's `system.ltx` tree and the text of its `class_registrator.script` (empty
    /// when the game has none).
    #[must_use]
    pub fn new(ltx: Arc<Ltx>, class_registrator: &str) -> Self {
        let mut families = FxHashMap::default();
        parse_class_registrator(class_registrator, &mut families);
        for (clsid, f) in FALLBACK {
            families.entry((*clsid).to_owned()).or_insert(*f);
        }
        Self { ltx, families, cache: Mutex::new(FxHashMap::default()) }
    }

    /// An empty value is ABSENT: a bare `upgrades =` cancels an inherited upgrade root.
    fn read(&self, section: &str, key: &str) -> Option<&str> {
        if section.is_empty() {
            return None;
        }
        let s = self.ltx.section(section)?;
        let v = s.get(key)?.value.as_deref()?;
        let v = v.strip_suffix('"').unwrap_or(v);
        let v = v.strip_prefix('"').unwrap_or(v);
        (!v.is_empty()).then_some(v)
    }

    fn upgrades(&self, section: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let Some(roots) = self.read(section, "upgrades") else { return out };
        let mut queue = split(roots, ',');
        let mut seen: Vec<String> = Vec::new();
        let mut guard = 0;
        while guard < 512 {
            let Some(g) = queue.pop() else { break };
            guard += 1;
            let g = trim(&g).to_owned();
            if g.is_empty() || seen.contains(&g) {
                continue;
            }
            seen.push(g.clone());
            let Some(els) = self.read(&g, "elements") else { continue };
            for el in split(els, ',') {
                if el.is_empty() {
                    continue;
                }
                if !out.contains(&el) {
                    out.push(el.clone());
                }
                if let Some(eff) = self.read(&el, "effects") {
                    queue.extend(split(eff, ','));
                }
            }
        }
        out
    }

    fn compute(&self, section: &str) -> Caps {
        let mut c = Caps::default();
        if self.ltx.section(section).is_none() {
            // unknown section: everything offered, nothing warned
            return c;
        }
        let fam = self.read(section, "class").and_then(|cls| self.families.get(cls)).copied();
        // ceilings are facts of the section whatever the family says
        c.mag_size = self.read(section, "ammo_mag_size").map_or(0, |v| i32::try_from(c_atoi(v)).unwrap_or(0));
        c.box_size = self.read(section, "box_size").map_or(0, |v| i32::try_from(c_atoi(v)).unwrap_or(0));
        if let Some(ac) = self.read(section, "ammo_class") {
            c.ammo_class = split(ac, ',');
        }
        let Some(fam) = fam else { return c };
        c.known = true;
        c.mask = 0;
        match self.read(section, "use_condition") {
            Some(uc) => {
                let l = uc.to_ascii_lowercase();
                if l == "true" || l == "on" || l == "1" {
                    c.mask |= CONDITION;
                }
            }
            None => {
                if matches!(
                    fam,
                    Family::WeaponMag
                        | Family::WeaponPlain
                        | Family::Wearable
                        | Family::Explosive
                        | Family::Artefact
                ) {
                    c.mask |= CONDITION;
                }
            }
        }
        if fam == Family::Ammo {
            c.mask |= AMMO_BOX | AMMO;
            c.mag_size = 0;
        } else if fam == Family::WeaponMag && c.mag_size > 0 {
            c.mask |= AMMO;
            if c.ammo_class.len() > 1 {
                c.mask |= AMMO_KIND;
            }
        }
        if self.read(section, "upgrades").is_some() {
            c.mask |= UPGRADES;
            c.upgrades = self.upgrades(section);
        }
        c
    }

    /// What `section` can carry.
    pub fn of(&self, section: &str) -> Arc<Caps> {
        if section.is_empty() {
            return Arc::new(Caps::default());
        }
        if let Some(c) = self.cache.lock().get(section) {
            return c.clone();
        }
        let c = Arc::new(self.compute(section));
        self.cache.lock().insert(section.to_owned(), c.clone());
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_come_from_the_registrator() {
        let mut f = FxHashMap::default();
        parse_class_registrator(
            "-- cs_register(factory, \"a\", \"se_ammo\", \"OLD\", \"x\")\n  cs_register(factory, \"CWeaponAK74\", \"se_item.se_weapon_magazined\", \"WP_AK74\", \"wpn\")\ncs_register(f, \"k\", \"se_item.se_weapon\", \"WP_KNIFE\", \"k\")\n",
            &mut f,
        );
        assert_eq!(f.get("WP_AK74"), Some(&Family::WeaponMag));
        assert_eq!(f.get("WP_KNIFE"), Some(&Family::WeaponPlain));
        assert!(!f.contains_key("OLD"));
        assert_eq!(cap_of_key("ammo"), AMMO);
        assert_eq!(cap_of_key("section"), ALL);
        assert!(Caps::default().has(UPGRADES));
    }
}
