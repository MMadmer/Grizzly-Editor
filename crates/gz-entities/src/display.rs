//! How a property is named outside the sheet: a stable key for files and commands
//! ([`Prop::key`]) and the plain-English label, section and unit the editor shows
//! ([`Prop::display`], [`word_of`] for a token's item).

use crate::model::{addon_flags, alife_flags, lamp_flags, skeleton_flags};
use crate::props::{Field, LampField, Prop};

fn alife_flag(bit: u32) -> String {
    let name = match bit {
        alife_flags::USE_SWITCHES => "use_switches",
        alife_flags::SWITCH_ONLINE => "switch_online",
        alife_flags::SWITCH_OFFLINE => "switch_offline",
        alife_flags::INTERACTIVE => "interactive",
        alife_flags::VISIBLE_FOR_AI => "visible_for_ai",
        alife_flags::USEFUL_FOR_AI => "useful_for_ai",
        alife_flags::OFFLINE_NO_MOVE => "offline_no_move",
        alife_flags::USED_AI_LOCATIONS => "used_ai_locations",
        alife_flags::GROUP_BEHAVIOUR => "group_behaviour",
        alife_flags::CAN_SAVE => "can_save",
        alife_flags::VISIBLE_FOR_MAP => "visible_for_map",
        alife_flags::USE_SMART_TERRAINS => "use_smart_terrains",
        alife_flags::CHECK_FOR_SEPARATOR => "check_for_separator",
        other => return format!("alife_flags.bit_{}", other.trailing_zeros()),
    };
    format!("alife_flags.{name}")
}

fn lamp_flag(bit: u16) -> String {
    let name = match bit {
        lamp_flags::PHYSIC => "physic",
        lamp_flags::CAST_SHADOW => "cast_shadow",
        lamp_flags::R1 => "r1",
        lamp_flags::R2 => "r2",
        lamp_flags::TYPE_SPOT => "spot",
        lamp_flags::POINT_AMBIENT => "ambient",
        lamp_flags::VOLUMETRIC => "volumetric",
        other => return format!("lamp.flags.bit_{}", other.trailing_zeros()),
    };
    format!("lamp.flags.{name}")
}

fn lamp_field(f: LampField) -> &'static str {
    match f {
        LampField::Color => "color",
        LampField::Brightness => "brightness",
        LampField::ColorAnimator => "color_animator",
        LampField::Range => "range",
        LampField::VirtualSize => "virtual_size",
        LampField::Texture => "texture",
        LampField::MainBone => "main_bone",
        LampField::ConeAngle => "cone_angle",
        LampField::AmbientRadius => "ambient_radius",
        LampField::AmbientPower => "ambient_power",
        LampField::AmbientTexture => "ambient_texture",
        LampField::AmbientBone => "ambient_bone",
        LampField::VolumetricQuality => "volumetric_quality",
        LampField::VolumetricIntensity => "volumetric_intensity",
        LampField::VolumetricDistance => "volumetric_distance",
        LampField::FixedBones => "fixed_bones",
        LampField::GlowRadius => "glow_radius",
        LampField::GlowTexture => "glow_texture",
        LampField::Health => "health",
    }
}

/// `Hit power` -> `hit_power`.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_owned()
}

/// Labels the sheet spells in a way a reader should not see.
const RENAMES: &[(&str, &str)] = &[
    ("Story ID (script)", "Script Story ID"),
    ("Used AI locations", "Uses AI Locations"),
    ("check for separator", "Checks for Separator"),
    ("npc profile", "Character Profile"),
    ("Item condition", "Condition"),
    ("Podstvolnik", "Grenade Launcher"),
    ("Ammo: in magazine", "Ammo in Magazine"),
    ("Ammo: left", "Ammo Left"),
    ("Ammo type:", "Ammo Type"),
    ("Anomaly value:", "Anomaly Value"),
    ("Info portion :", "Info Portion"),
    ("No move in offline", "Stays Put Offline"),
    ("Use smart terrain tasks", "Uses Smart Terrain Tasks"),
    ("Artefact spawn places count", "Artefact Spawn Places"),
    ("Level to change", "Level"),
    ("Level point to change", "Point"),
    ("Allow R1", "Allowed in R1"),
    ("Allow R2", "Allowed in R2"),
    ("Allow Ambient", "Ambient Light"),
    ("in space restrictions", "Inside Restrictions"),
    ("out space restrictions", "Outside Restrictions"),
    ("on/off mode", "On and Off"),
    ("Kept as bytes", "Kept As Is"),
];

/// Words Title Case leaves small inside a label.
const SMALL: &[&str] = &["a", "an", "and", "as", "at", "by", "for", "in", "of", "on", "or", "the", "to"];

/// Title Case: every word capitalised but the small ones inside; a word that already holds a
/// capital (`AI`, `ID`, `R1`) is kept.
#[must_use]
pub fn title_case(text: &str) -> String {
    let mut out = String::new();
    for (i, w) in text.split_whitespace().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if w.chars().any(char::is_uppercase) || (i > 0 && SMALL.contains(&w)) {
            out.push_str(w);
        } else {
            let mut c = w.chars();
            if let Some(f) = c.next() {
                out.extend(f.to_uppercase());
                out.push_str(c.as_str());
            }
        }
    }
    out
}

/// One segment of a sheet path as the editor shows it, and the unit it names.
fn segment(seg: &str) -> (String, &'static str) {
    if let Some((_, to)) = RENAMES.iter().find(|(from, _)| *from == seg) {
        return ((*to).to_owned(), "");
    }
    let (text, unit) = match seg.strip_suffix(" (sec)") {
        Some(t) => (t, "s"),
        None => (seg, ""),
    };
    (title_case(text.trim_end_matches([':', ' '])), unit)
}

/// A token item as the editor shows it: `NOT A restrictor` reads None, `team deathmatch` Team
/// Deathmatch.
#[must_use]
pub fn word_of(item: &str) -> String {
    match item {
        "NOT A restrictor" => "None".to_owned(),
        "NONE default restrictor" => "Default None".to_owned(),
        "OUT default restrictor" => "Default Outside".to_owned(),
        "IN default restrictor" => "Default Inside".to_owned(),
        other => title_case(other),
    }
}

impl Prop {
    /// The key files and commands name the property by: stable whatever the sheet calls it
    /// (`story_id`, `trader.money`, `lamp.flags.cast_shadow`).
    #[must_use]
    pub fn key(&self) -> String {
        let s = |t: &str| t.to_owned();
        match &self.field {
            Field::None => {
                format!("info.{}", slug(self.path.split_once('\\').map_or(self.path.as_str(), |(_, r)| r)))
            }
            Field::CustomData => s("custom_data"),
            Field::ScriptStory => s("script_story_id"),
            Field::AlifeFlag(bit) => alife_flag(*bit),
            Field::StoryId => s("story_id"),
            Field::SpawnStoryId => s("spawn_story_id"),
            Field::Visual => s("visual"),
            Field::Animation => s("startup_animation"),
            Field::VisualFlag(_) => s("visual_flags.obstacle"),
            Field::Motion => s("motion"),
            Field::GpLocation(i) => format!("graph_point.location_{i}"),
            Field::GpLevel => s("graph_point.level"),
            Field::GpPoint => s("graph_point.point"),
            Field::Money => s("trader.money"),
            Field::TraderFlag(_) => s("trader.infinite_ammo"),
            Field::Profile => s("trader.character_profile"),
            Field::GroupCount => s("group.count"),
            Field::RestrictorType => s("restrictor.restrictor_type"),
            Field::LcLevel => s("level_changer.level"),
            Field::LcPoint => s("level_changer.point"),
            Field::LcSilent => s("level_changer.silent"),
            Field::PhType => s("physic.kind"),
            Field::PhMass => s("physic.mass"),
            Field::SkeletonFlag(bit) if *bit == skeleton_flags::ACTIVE => s("skeleton.active"),
            Field::SkeletonFlag(bit) => format!("skeleton.bit_{}", bit.trailing_zeros()),
            Field::PhFixedBones => s("physic.fixed_bones"),
            Field::Lamp(f) => format!("lamp.{}", lamp_field(*f)),
            Field::LampFlag(bit) => lamp_flag(*bit),
            Field::EngineSound => s("helicopter.engine_sound"),
            Field::CarHealth => s("car.health"),
            Field::BreakableHealth => s("breakable.health"),
            Field::TeamBase => s("team_base.team"),
            Field::Condition => s("item.condition"),
            Field::WpnAmmoType => s("weapon.ammo_type"),
            Field::WpnElapsed => s("weapon.ammo_elapsed"),
            Field::WpnAddon(bit) => match *bit {
                addon_flags::SCOPE => s("weapon.addons.scope"),
                addon_flags::SILENCER => s("weapon.addons.silencer"),
                addon_flags::GRENADE_LAUNCHER => s("weapon.addons.grenade_launcher"),
                other => format!("weapon.addons.bit_{}", other.trailing_zeros()),
            },
            Field::AmmoLeft => s("ammo.elapsed"),
            Field::AnomalyValue => s("artefact.anomaly_value"),
            Field::DocInfo => s("document.info_portion"),
            Field::ZoneShift => s("zone.start_time_shift"),
            Field::ZoneEnabled => s("zone.enabled_time"),
            Field::ZoneDisabled => s("zone.disabled_time"),
            Field::ZoneRadius => s("zone.offline_interactive_radius"),
            Field::ZoneArtefactCount => s("zone.artefact_spawn_count"),
            Field::AttackAnimation => s("zone.attack_animation"),
            Field::CrTeam => s("creature.team"),
            Field::CrSquad => s("creature.squad"),
            Field::CrGroup => s("creature.group"),
            Field::CrHealth => s("creature.health"),
            Field::OutRestrictions => s("monster.out_restrictions"),
            Field::InRestrictions => s("monster.in_restrictions"),
            Field::Rat(_) => format!("rat.{}", slug(self.path.rsplit_once('\\').map_or("", |(_, l)| l))),
            Field::Zombie(_) => {
                format!("zombie.{}", slug(self.path.rsplit_once('\\').map_or("", |(_, l)| l)))
            }
            Field::CoverHold => s("cover.hold_position_time"),
            Field::CoverDescription => s("cover.description"),
            Field::CoverEnter => s("cover.enter_min_enemy_distance"),
            Field::CoverExit => s("cover.exit_min_enemy_distance"),
            Field::CoverCombat => s("cover.is_combat_cover"),
            Field::CoverCanFire => s("cover.can_fire"),
            Field::Loophole(id) => format!("cover.loopholes.{id}"),
        }
    }

    /// The section the editor shows the property under (`\`-separated, may be empty), its
    /// label in Title Case and its unit.
    #[must_use]
    pub fn display(&self) -> (String, String, &'static str) {
        // every path but the group's count starts with the spawn section
        let rest = match self.field {
            Field::GroupCount => self.path.as_str(),
            _ => self.path.split_once('\\').map_or(self.path.as_str(), |(_, r)| r),
        };
        let mut parts: Vec<(String, &'static str)> = rest.split('\\').map(segment).collect();
        let (label, unit) = parts.pop().unwrap_or_default();
        let group: Vec<String> = parts.into_iter().map(|(t, _)| t).collect();
        let unit = match (&self.field, unit) {
            (_, u) if !u.is_empty() => u,
            (
                Field::Lamp(
                    LampField::Range
                    | LampField::AmbientRadius
                    | LampField::GlowRadius
                    | LampField::VirtualSize,
                ),
                _,
            )
            | (Field::ZoneRadius | Field::CoverEnter | Field::CoverExit, _) => "m",
            (Field::Lamp(LampField::ConeAngle), _) => "\u{b0}",
            (Field::CoverHold, _) => "s",
            (Field::PhMass, _) => "kg",
            _ => "",
        };
        (group.join("\\"), label, unit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_read_as_plain_english() {
        assert_eq!(segment("Story ID (script)").0, "Script Story ID");
        assert_eq!(segment("Shift time (sec)"), ("Shift Time".to_owned(), "s"));
        assert_eq!(segment("can fire").0, "Can Fire");
        assert_eq!(segment("Personal\u{20}Field of view").0, "Personal Field of View");
        assert_eq!(segment("Allow R1").0, "Allowed in R1");
        assert_eq!(word_of("NOT A restrictor"), "None");
        assert_eq!(word_of("team deathmatch"), "Team Deathmatch");
        assert_eq!(slug("Hit power"), "hit_power");
    }
}
