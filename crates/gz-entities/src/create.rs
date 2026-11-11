//! The constructors: what `xrFactory_Create` makes of a spawn section.
//!
//! Every read of the section the C++ constructors make is made here, in their order. A key a
//! constructor reads without asking first is required: the engine stops on a missing one, and
//! this returns [`EntityError::MissingKey`].

use smol_str::SmolStr;

use crate::class::{Class, ClassInfo};
use crate::entity::Header;
use crate::model::{Object, TRADER_INFINITE_AMMO, alife_flags, visual_name};
use crate::packet::TextEncoding;
use crate::script::Script;
use crate::settings::Settings;
use crate::{EntityError, Result};

const HIT_TYPES: [&str; 11] = [
    "burn",
    "light_burn",
    "shock",
    "strike",
    "wound",
    "radiation",
    "telepatic",
    "fire_wound",
    "chemical_burn",
    "explosion",
    "wound_2",
];

fn need<'a>(s: &'a dyn Settings, section: &str, key: &str) -> Result<&'a str> {
    s.r_string(section, key)
        .ok_or_else(|| EntityError::MissingKey { section: SmolStr::new(section), key: SmolStr::new(key) })
}

fn need_f32(s: &dyn Settings, section: &str, key: &str) -> Result<f32> {
    need(s, section, key).map(|v| crate::packet::atof(v) as f32)
}

fn need_i32(s: &dyn Settings, section: &str, key: &str) -> Result<i32> {
    need(s, section, key).map(crate::packet::atoi)
}

fn need_u32(s: &dyn Settings, section: &str, key: &str) -> Result<u32> {
    need_i32(s, section, key).map(|v| v as u32)
}

/// `ALife::g_tfString2HitType`, which stops the engine on a name it does not know.
fn need_hit_type(s: &dyn Settings, section: &str, key: &str) -> Result<()> {
    let value = need(s, section, key)?;
    if HIT_TYPES.iter().any(|t| value.eq_ignore_ascii_case(t)) {
        Ok(())
    } else {
        Err(EntityError::BadValue {
            section: SmolStr::new(section),
            key: SmolStr::new(key),
            value: SmolStr::new(value),
        })
    }
}

/// `script_server_object_version()`: `[script] current_server_entity_version`.
pub(crate) fn current_script_version(settings: &dyn Settings) -> u16 {
    settings.r_u16("script", "current_server_entity_version").unwrap_or(0)
}

/// Makes the parts the class is built from, so that every writer finds what it writes.
pub(crate) fn ensure_parts(o: &mut Object, class: Class) {
    if class.is_item() {
        o.item_mut();
    }
    if class.is_weapon() {
        o.weapon_mut();
    }
    if class.is_trader() {
        o.trader_mut();
    }
    if class.is_creature() {
        o.creature_mut();
    }
    if class.is_monster() {
        o.monster_mut();
    }
    if class.is_restrictor() {
        o.restrictor_mut();
    }
    if class.is_zone() {
        o.zone_mut();
    }
    match class {
        Class::GraphPoint => {
            o.graph_point_mut();
        }
        Class::Temporary => {
            o.temporary_mut();
        }
        Class::LevelChanger => {
            o.level_changer_mut();
        }
        Class::ObjectPhysic => {
            o.physic_mut();
        }
        Class::HangingLamp => {
            o.lamp_mut();
        }
        Class::Helicopter => {
            o.helicopter_mut();
        }
        Class::Car => {
            o.car_mut();
        }
        Class::Breakable => {
            o.breakable_mut();
        }
        Class::Climable => {
            o.climable_mut();
        }
        Class::StationaryMgun => {
            o.mgun_mut();
        }
        Class::TeamBaseZone => {
            o.team_base_mut();
        }
        Class::InventoryBox => {
            o.inventory_box_mut();
        }
        Class::ItemTorch => {
            o.torch_mut();
        }
        Class::ItemAmmo => {
            o.ammo_mut();
        }
        Class::ItemArtefact => {
            o.artefact_mut();
        }
        Class::ItemPda => {
            o.pda_mut();
        }
        Class::ItemDocument => {
            o.document_mut();
        }
        Class::CreatureActor => {
            o.actor_mut();
        }
        Class::MonsterRat => {
            o.rat_mut();
        }
        Class::MonsterZombie => {
            o.zombie_mut();
        }
        Class::HumanStalker => {
            o.human_mut();
        }
        Class::OnlineOfflineGroup => {
            o.squad_mut();
        }
        Class::SmartCover => {
            o.cover_mut();
        }
        Class::MonsterBaseGroup => {
            o.group_mut();
        }
        _ => {}
    }
}

/// `CSE_ALifeTraderAbstract::init`: the custom data gains a `[game_info]` section.
fn trader_init(o: &mut Object) {
    o.alife.ini_string = SmolStr::new(format!("{}\r\n[game_info]\r\n", o.alife.ini_string));
}

/// `CSE_ALifeDynamicObjectVisual` and the classes that repeat it: the section's `visual`.
fn section_visual(o: &mut Object, s: &dyn Settings, section: &str) {
    if let Some(visual) = s.r_string(section, "visual") {
        o.visual.set(visual);
    }
}

fn inventory_item(o: &mut Object, s: &dyn Settings, section: &str) -> Result<()> {
    let mass = need_f32(s, section, "inv_weight")?;
    let cost = need_u32(s, section, "cost")?;
    let item = o.item_mut();
    item.mass = mass;
    item.cost = cost;
    if let Some(condition) = s.r_float(section, "condition") {
        item.condition = condition;
    }
    Ok(())
}

fn trader_abstract(o: &mut Object, s: &dyn Settings, section: &str) -> Result<()> {
    let max_item_mass = need_f32(s, section, "max_item_mass")?;
    let trader = o.trader_mut();
    if let Some(money) = s.r_u32(section, "money") {
        trader.money = money;
    }
    trader.max_item_mass = max_item_mass;
    if let Some(profile) = s.r_string(section, "character_profile") {
        trader.character_profile = SmolStr::new(profile);
    }
    Ok(())
}

fn creature(o: &mut Object, s: &dyn Settings, section: &str) -> Result<()> {
    need_u32(s, section, "ef_creature_type")?;
    o.creature_mut();
    Ok(())
}

fn monster(o: &mut Object, s: &dyn Settings, section: &str) -> Result<()> {
    creature(o, s, section)?;
    need_f32(s, section, "going_speed")?;
    need(s, section, "terrain")?;
    need_f32(s, section, "MaxHealthValue")?;
    if s.line_exist(section, "hit_power") {
        need_hit_type(s, section, "hit_type")?;
    }
    need_f32(s, section, "eye_range")?;
    let graph_id = o.alife.graph_id;
    let m = o.monster_mut();
    m.next_graph_id = graph_id;
    m.prev_graph_id = graph_id;
    if let Some(restrictions) = s.r_string(section, "SpaceRestrictionSection") {
        m.space_restriction_section = SmolStr::new(restrictions);
    }
    Ok(())
}

fn custom_zone(o: &mut Object, s: &dyn Settings, section: &str) -> Result<()> {
    space_restrictor(o);
    // the game: `read_if_exists<float>(caSection, "min_start_power", 1.0f)`
    o.zone_mut().max_power = s.r_float(section, "min_start_power").unwrap_or(1.0);
    if s.line_exist(section, "hit_type") {
        need_hit_type(s, section, "hit_type")?;
    }
    Ok(())
}

fn space_restrictor(o: &mut Object) {
    o.alife.flags &= !(alife_flags::USE_SWITCHES | alife_flags::USED_AI_LOCATIONS);
    o.alife.flags |= alife_flags::CHECK_FOR_SEPARATOR;
    o.restrictor_mut().restrictor_type = 0;
}

fn no_switches(o: &mut Object) {
    o.alife.flags &= !(alife_flags::USE_SWITCHES | alife_flags::SWITCH_OFFLINE);
}

fn weapon(o: &mut Object, s: &dyn Settings, section: &str) -> Result<()> {
    need_f32(s, section, "hit_power")?;
    need_hit_type(s, section, "hit_type")?;
    need(s, section, "ammo_class")?;
    section_visual(o, s, section);
    let scope = need_i32(s, section, "scope_status")?;
    let silencer = need_i32(s, section, "silencer_status")?;
    let launcher = need_i32(s, section, "grenade_launcher_status")?;
    let w = o.weapon_mut();
    w.scope_status = scope;
    w.silencer_status = silencer;
    w.grenade_launcher_status = launcher;
    w.mag_size = s.r_u32(section, "ammo_mag_size").unwrap_or(0);
    Ok(())
}

/// The header and the parts of a new entity of `info`'s class.
pub(crate) fn construct(info: &ClassInfo, section: &str, s: &dyn Settings) -> Result<(Header, Object)> {
    let class = info.class;
    // `CSE_ALifeGroupTemplate` hands its base the monster's section
    let section = match class {
        Class::MonsterBaseGroup => s.r_string(section, "monster_section").unwrap_or(section),
        _ => section,
    };

    let mut h = Header {
        section: SmolStr::new(section),
        name: SmolStr::default(),
        game_id: 0,
        rp: 0xFE,
        position: glam::Vec3::ZERO,
        angle: glam::Vec3::ZERO,
        respawn_time: 0,
        id: u16::MAX,
        id_parent: u16::MAX,
        id_phantom: u16::MAX,
        flags: 0,
        version: 0,
        game_type: u16::MAX,
        script_version: 0,
        write_script_version: current_script_version(s),
        client_data: Vec::new(),
        // oracle: `m_tSpawnID` is left uninitialised for the classes below `CSE_ALifeObject`
        spawn_id: if class.is_alife() { u16::MAX } else { 0 },
        text: TextEncoding::Cp1251,
    };
    let mut o = Object::default();

    // CSE_Abstract: the custom data file the section names
    if let Some(file) = s.r_string(section, "custom_data")
        && let Some(text) = s.config_text(file)
    {
        let text = text.split('\0').next().unwrap_or("");
        o.alife.ini_string = SmolStr::new(text);
    }

    if class.has_visual() && class != Class::AbstractVisual && class != Class::ZoneVisual {
        // CSE_ALifeDynamicObjectVisual
        section_visual(&mut o, s, section);
    }

    match class {
        Class::Spectator | Class::GraphPoint | Class::Temporary => {}
        Class::AbstractVisual => o.visual.name = visual_name(section),
        Class::DynamicObjectVisual | Class::MountedWeapon | Class::StationaryMgun | Class::InventoryBox => {}
        Class::PhSkeletonObject | Class::Projector | Class::HangingLamp | Class::Breakable => {
            no_switches(&mut o)
        }
        Class::SpaceRestrictor | Class::LevelChanger | Class::SmartZone | Class::TeamBaseZone => {
            space_restrictor(&mut o);
        }
        Class::ObjectPhysic => {
            if s.section_exist(section)
                && s.line_exist(section, "visual")
                && let Some(animation) = s.r_string(section, "startup_animation")
            {
                o.visual.startup_animation = SmolStr::new(animation);
            }
            if let Some(bones) = s.r_string(section, "fixed_bones") {
                o.physic_mut().fixed_bones = SmolStr::new(bones);
            }
            no_switches(&mut o);
            o.alife.flags &= !alife_flags::USED_AI_LOCATIONS;
        }
        Class::Helicopter => {
            no_switches(&mut o);
            o.alife.flags &= !alife_flags::INTERACTIVE;
        }
        Class::Car => no_switches(&mut o),
        Class::Climable => {}
        Class::Item
        | Class::ItemTorch
        | Class::ItemArtefact
        | Class::ItemPda
        | Class::ItemDocument
        | Class::ItemGrenade
        | Class::ItemExplosive
        | Class::ItemHelmet => inventory_item(&mut o, s, section)?,
        Class::ItemAmmo => {
            inventory_item(&mut o, s, section)?;
            // the game: the box may start partly used (`box_current`)
            let box_size = need_i32(s, section, "box_size")? as u16;
            let current = s.r_s32(section, "box_current").unwrap_or(i32::from(box_size));
            let ammo = o.ammo_mut();
            ammo.box_size = box_size;
            ammo.elapsed = current.clamp(0, i32::from(box_size)) as u16;
        }
        Class::ItemWeapon
        | Class::ItemWeaponMagazined
        | Class::ItemWeaponMagazinedWGL
        | Class::ItemWeaponShotGun
        | Class::ItemWeaponAutoShotGun => {
            inventory_item(&mut o, s, section)?;
            weapon(&mut o, s, section)?;
        }
        Class::ItemDetector => {
            inventory_item(&mut o, s, section)?;
            need_u32(s, section, "ef_detector_type")?;
        }
        Class::ItemBolt => {
            inventory_item(&mut o, s, section)?;
            // the game: a bolt that is not infinite is an ordinary saved item
            if s.r_bool(section, "infinite").unwrap_or(true) {
                no_switches(&mut o);
                o.alife.flags &= !alife_flags::CAN_SAVE;
            }
        }
        Class::ItemCustomOutfit => {
            inventory_item(&mut o, s, section)?;
            need_u32(s, section, "ef_equipment_type")?;
        }
        Class::Trader => {
            trader_abstract(&mut o, s, section)?;
            trader_init(&mut o);
        }
        Class::CustomZone | Class::AnomalousZone | Class::TorridZone => custom_zone(&mut o, s, section)?,
        Class::ZoneVisual => {
            custom_zone(&mut o, s, section)?;
            // CSE_Visual(caSection), then the section's own visual
            o.visual.name = visual_name(section);
            section_visual(&mut o, s, section);
        }
        Class::CreatureActor => {
            creature(&mut o, s, section)?;
            trader_abstract(&mut o, s, section)?;
            trader_init(&mut o);
        }
        Class::CreatureCrow | Class::CreaturePhantom => {
            creature(&mut o, s, section)?;
            no_switches(&mut o);
        }
        Class::MonsterRat => {
            monster(&mut o, s, section)?;
            inventory_item(&mut o, s, section)?;
            o.creature_mut().health = 5.0;
        }
        Class::MonsterZombie => {
            monster(&mut o, s, section)?;
            o.creature_mut().health = 200.0;
        }
        Class::MonsterBase | Class::PsyDogPhantom | Class::MonsterBaseGroup => {
            monster(&mut o, s, section)?;
            // `set_visual(pSettings->r_string(caSection, "visual"))`
            need(s, section, "visual")?;
        }
        Class::HumanStalker => {
            trader_abstract(&mut o, s, section)?;
            monster(&mut o, s, section)?;
            o.trader_mut().flags |= TRADER_INFINITE_AMMO;
            trader_init(&mut o);
        }
        Class::OnlineOfflineGroup => o.alife.flags &= !alife_flags::USED_AI_LOCATIONS,
        Class::SmartCover => {
            let enter = need_f32(s, section, "enter_min_enemy_distance")?;
            let exit = need_f32(s, section, "exit_min_enemy_distance")?;
            let combat = crate::settings::is_bool_true(need(s, section, "is_combat_cover")?);
            // the game reads `can_fire` only when it is there
            let can_fire = combat || s.r_bool(section, "can_fire").unwrap_or(false);
            let cover = o.cover_mut();
            cover.enter_min_enemy_distance = enter;
            cover.exit_min_enemy_distance = exit;
            cover.is_combat_cover = u8::from(combat);
            cover.can_fire = u8::from(can_fire);
        }
    }

    ensure_parts(&mut o, class);
    o.script = Script::new(info.script);
    h.flags = 0;
    Ok((h, o))
}

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::settings::MapSettings;

    /// A `system.ltx` with one section of every class family.
    pub(crate) fn settings() -> MapSettings {
        let item = [("inv_weight", "0.5"), ("cost", "100"), ("visual", "Dynamics\\Item.ogf")];
        let creature = [
            ("ef_creature_type", "1"),
            ("going_speed", "1.5"),
            ("terrain", "stalker_terrain"),
            ("MaxHealthValue", "200"),
            ("eye_range", "100"),
            ("visual", "actors\\x\\x.ogf"),
        ];
        let weapon = [
            ("hit_power", "0.5"),
            ("hit_type", "fire_wound"),
            ("ammo_class", "ammo_9x18"),
            ("scope_status", "2"),
            ("silencer_status", "2"),
            ("grenade_launcher_status", "0"),
            ("ammo_mag_size", "8"),
        ];
        let mut s = MapSettings::new()
            .section("script", &[("current_server_entity_version", "12")])
            .section("spectator", &[("class", "SPECT")])
            .section("graph_point", &[("class", "AI_GRAPH")])
            .section("rocket", &[("class", "G_RPG7")])
            .section("script_object", &[("class", "SCRPTOBJ"), ("visual", "dynamics\\box.ogf")])
            .section("skeleton_object", &[("class", "P_SKELET"), ("visual", "dynamics\\bones.ogf")])
            .section("space_restrictor", &[("class", "SPC_RS_S")])
            .section("level_changer", &[("class", "LVL_CHNG")])
            .section("old_level_changer", &[("class", "LVLCHNGR")])
            .section("smart_terrain", &[("class", "SMRTTRRN")])
            .section("smart_zone", &[("class", "SMRTZONE")])
            .section(
                "physic_object",
                &[
                    ("class", "O_PHYS_S"),
                    ("visual", "dynamics\\Barrel.ogf"),
                    ("fixed_bones", "link"),
                    ("startup_animation", "idle"),
                ],
            )
            .section("lamp", &[("class", "SO_HLAMP"), ("visual", "dynamics\\lamp.ogf")])
            .section("projector", &[("class", "O_SEARCH")])
            .section("helicopter", &[("class", "C_HLCP_S"), ("visual", "dynamics\\heli.ogf")])
            .section("car", &[("class", "SCRPTCAR"), ("visual", "dynamics\\niva.ogf")])
            .section("breakable", &[("class", "O_BRKBL"), ("visual", "dynamics\\glass.ogf")])
            .section("climable", &[("class", "O_CLMBL")])
            .section("mounted", &[("class", "W_STMGUN"), ("visual", "dynamics\\mgun.ogf")])
            .section("team_base", &[("class", "Z_TEAMBS")])
            .section(
                "inventory_box",
                &[
                    ("class", "S_INVBOX"),
                    ("visual", "dynamics\\box.ogf"),
                    ("custom_data", "scripts\\box.ltx"),
                ],
            )
            .section("zone", &[("class", "Z_ZONE")])
            .section("anomaly", &[("class", "ZS_MBALD"), ("hit_type", "strike"), ("min_start_power", "0.25")])
            .section("torrid", &[("class", "ZS_TORRD")])
            .section("zone_bfuzz.x", &[("class", "ZS_BFUZZ"), ("visual", "dynamics\\Fuzz.ogf")])
            .section(
                "crow",
                &[("class", "AI_CROW"), ("ef_creature_type", "2"), ("visual", "monsters\\crow.ogf")],
            )
            .section("phantom", &[("class", "AI_PHANT"), ("ef_creature_type", "2")])
            .section("squad", &[("class", "ON_OFF_S")])
            .section("plain_squad", &[("class", "ON_OFF_G")])
            .section(
                "smart_cover",
                &[
                    ("class", "SMRT_C_S"),
                    ("enter_min_enemy_distance", "15"),
                    ("exit_min_enemy_distance", "10"),
                    ("is_combat_cover", "true"),
                ],
            )
            .section(
                "animpoint",
                &[
                    ("class", "SMRTCOVR"),
                    ("enter_min_enemy_distance", "1"),
                    ("exit_min_enemy_distance", "2"),
                    ("is_combat_cover", "false"),
                ],
            )
            .section("unknown_thing", &[("class", "NOCLASS")])
            .file("scripts\\box.ltx", "[spawn]\r\nmedkit = 2\r\n");

        let with = |base: &[(&str, &str)], more: &[(&str, &str)]| -> Vec<(String, String)> {
            base.iter().chain(more).map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
        };
        let mut add = |name: &str, lines: Vec<(String, String)>| {
            for (k, v) in lines {
                s.set(name, &k, &v);
            }
        };
        add("medkit", with(&item, &[("class", "II_MEDKI")]));
        add("bread", with(&item, &[("class", "S_FOOD")]));
        add("torch", with(&item, &[("class", "TORCH_S")]));
        add("ammo_9x18", with(&item, &[("class", "AMMO_S"), ("box_size", "20")]));
        add("ammo_half", with(&item, &[("class", "AMMO"), ("box_size", "20"), ("box_current", "7")]));
        add("knife", with(&with_refs(&item, &weapon), &[("class", "WP_KNIFE")]));
        add("pistol", with(&with_refs(&item, &weapon), &[("class", "WP_PM")]));
        add("rifle", with(&with_refs(&item, &weapon), &[("class", "WP_AK74")]));
        add("shotgun", with(&with_refs(&item, &weapon), &[("class", "WP_BM16")]));
        add("auto_shotgun", with(&with_refs(&item, &weapon), &[("class", "WP_ASHTG")]));
        add("detector", with(&item, &[("class", "DET_SIMP"), ("ef_detector_type", "1")]));
        add("artefact", with(&item, &[("class", "SCRPTART")]));
        add("plain_artefact", with(&item, &[("class", "ARTEFACT")]));
        add("pda", with(&item, &[("class", "S_PDA")]));
        add("document", with(&item, &[("class", "II_DOC")]));
        add("grenade", with(&item, &[("class", "G_F1_S")]));
        add("explosive", with(&item, &[("class", "S_EXPLO")]));
        add("bolt", with(&item, &[("class", "II_BOLT")]));
        add("counted_bolt", with(&item, &[("class", "II_BOLT"), ("infinite", "false")]));
        add("outfit", with(&item, &[("class", "E_STLK"), ("ef_equipment_type", "3")]));
        add("helmet", with(&item, &[("class", "E_HLMET")]));
        add(
            "trader",
            vec![
                ("class".to_owned(), "AI_TRD_S".to_owned()),
                ("max_item_mass".to_owned(), "1000".to_owned()),
                ("money".to_owned(), "5000".to_owned()),
                ("visual".to_owned(), "actors\\trader.ogf".to_owned()),
            ],
        );
        add("actor", with(&creature, &[("class", "S_ACTOR"), ("max_item_mass", "50")]));
        add(
            "dog",
            with(&creature, &[("class", "SM_DOG_S"), ("SpaceRestrictionSection", "space_restrictor")]),
        );
        add("plain_dog", with(&creature, &[("class", "AI_DOG_R")]));
        add("phantom_dog", with(&creature, &[("class", "AI_DOG_F")]));
        add("rat", with(&with_refs(&creature, &item), &[("class", "AI_RAT")]));
        add(
            "flesh_group",
            vec![
                ("class".to_owned(), "AI_FLE_G".to_owned()),
                ("monster_section".to_owned(), "plain_dog".to_owned()),
            ],
        );
        add(
            "stalker",
            with(
                &creature,
                &[("class", "AI_STL_S"), ("max_item_mass", "50"), ("character_profile", "sim_default")],
            ),
        );
        s
    }

    fn with_refs<'a>(a: &[(&'a str, &'a str)], b: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        a.iter().chain(b).copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::settings;
    use super::*;
    use crate::class::class_of_section;
    use crate::model::lamp_flags;

    fn make(section: &str) -> (Header, Object) {
        let s = settings();
        let info = class_of_section(section, &s).unwrap();
        construct(info, section, &s).unwrap()
    }

    #[test]
    fn the_abstract_part_starts_as_the_constructor_leaves_it() {
        let (h, o) = make("medkit");
        assert_eq!((h.rp, h.id, h.id_parent, h.id_phantom), (0xFE, 0xFFFF, 0xFFFF, 0xFFFF));
        assert_eq!((h.game_type, h.spawn_id, h.version, h.write_script_version), (0xFFFF, 0xFFFF, 0, 12));
        assert_eq!(o.alife.flags, 0xffff_ffbf);
        assert_eq!(o.visual.name, "dynamics\\item");
        assert_eq!(o.item().map(|i| (i.mass, i.cost, i.condition)), Some((0.5, 100, 1.0)));
        assert_eq!(make("graph_point").0.spawn_id, 0);
    }

    #[test]
    fn flags_follow_the_class() {
        use alife_flags::*;
        assert_eq!(
            make("physic_object").1.alife.flags,
            0xffff_ffbf & !(USE_SWITCHES | SWITCH_OFFLINE | USED_AI_LOCATIONS)
        );
        assert_eq!(make("space_restrictor").1.alife.flags, 0xffff_ffbf & !(USE_SWITCHES | USED_AI_LOCATIONS));
        assert_eq!(
            make("helicopter").1.alife.flags,
            0xffff_ffbf & !(USE_SWITCHES | SWITCH_OFFLINE | INTERACTIVE)
        );
        assert_eq!(make("bolt").1.alife.flags, 0xffff_ffbf & !(USE_SWITCHES | SWITCH_OFFLINE | CAN_SAVE));
        assert_eq!(make("counted_bolt").1.alife.flags, 0xffff_ffbf);
        assert_eq!(make("squad").1.alife.flags, 0xffff_ffbf & !USED_AI_LOCATIONS);
        assert_eq!(
            make("lamp").1.lamp().map(|l| l.flags),
            Some(lamp_flags::TYPE_SPOT | lamp_flags::R1 | lamp_flags::R2)
        );
    }

    #[test]
    fn section_defaults_are_read() {
        let (_, o) = make("physic_object");
        assert_eq!(o.visual.name, "dynamics\\barrel");
        assert_eq!(o.visual.startup_animation, "idle");
        assert_eq!(o.physic().map(|p| p.fixed_bones.as_str()), Some("link"));

        assert_eq!(make("ammo_9x18").1.ammo().map(|a| (a.box_size, a.elapsed)), Some((20, 20)));
        assert_eq!(make("ammo_half").1.ammo().map(|a| (a.box_size, a.elapsed)), Some((20, 7)));
        assert_eq!(
            make("pistol").1.weapon().map(|w| (w.scope_status, w.mag_size, w.a_current)),
            Some((2, 8, 90))
        );
        assert_eq!(make("anomaly").1.zone().map(|z| z.max_power), Some(0.25));
        assert_eq!(make("zone").1.zone().map(|z| z.max_power), Some(1.0));
        // CSE_Visual(caSection) cuts the section name like a file name; the section's visual wins
        assert_eq!(make("zone_bfuzz.x").1.visual.name, "dynamics\\fuzz");
        assert_eq!(make("inventory_box").1.alife.ini_string, "[spawn]\r\nmedkit = 2\r\n");
        assert_eq!(make("smart_cover").1.cover().map(|c| (c.is_combat_cover, c.can_fire)), Some((1, 1)));
        assert_eq!(make("animpoint").1.cover().map(|c| (c.is_combat_cover, c.can_fire)), Some((0, 0)));
        assert_eq!(
            make("dog").1.monster().map(|m| m.space_restriction_section.as_str()),
            Some("space_restrictor")
        );
    }

    #[test]
    fn people_get_game_info_and_their_profile() {
        let (_, o) = make("stalker");
        assert_eq!(o.alife.ini_string, "\r\n[game_info]\r\n");
        let t = o.trader().unwrap();
        assert_eq!(
            (t.character_profile.as_str(), t.flags, t.max_item_mass),
            ("sim_default", TRADER_INFINITE_AMMO, 50.0)
        );
        assert_eq!(make("trader").1.trader().map(|t| t.money), Some(5000));
        assert_eq!(make("actor").1.alife.ini_string, "\r\n[game_info]\r\n");
        assert_eq!(make("rat").1.creature().map(|c| c.health), Some(5.0));
    }

    #[test]
    fn a_group_is_built_from_its_monster_section() {
        let (h, o) = make("flesh_group");
        assert_eq!(h.section, "plain_dog");
        assert!(o.group().is_some() && o.monster().is_some());
    }

    #[test]
    fn a_missing_required_key_is_an_error() {
        let s = settings().section("broken", &[("class", "II_MEDKI"), ("inv_weight", "1")]);
        let info = class_of_section("broken", &s).unwrap();
        assert!(
            matches!(construct(info, "broken", &s), Err(EntityError::MissingKey { key, .. }) if key == "cost")
        );
        let s = settings().section("odd", &[("class", "Z_ZONE"), ("hit_type", "tickle")]);
        let info = class_of_section("odd", &s).unwrap();
        assert!(matches!(construct(info, "odd", &s), Err(EntityError::BadValue { .. })));
    }
}
