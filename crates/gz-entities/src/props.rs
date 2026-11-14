//! The property sheet: what each class lists, with typed get and set.
//!
//! Paths are the property keys without the prefix the caller adds: almost every one
//! starts with the spawn section (`stalker\ALife\Story ID`); a few are bare ones
//! (`Game Type`, `ALife\Count`). Lists the game builds at run time (story ids, levels,
//! character profiles, smart cover descriptions) are named by [`ListKind`] and resolved by the
//! caller; what the scripts know (smart covers) comes in through [`PropContext`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::class::{Class, ClassInfo};
use crate::entity::{Header, OpaqueReason};
use crate::model::{
    ADDON_ATTACHABLE, Object, TRADER_INFINITE_AMMO, VISUAL_OBSTACLE, addon_flags, alife_flags, lamp_flags,
    skeleton_flags,
};
use crate::script::Script;
use crate::{EntityError, Result};

/// A property's value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropValue {
    Bool(bool),
    Int(i64),
    Float(f32),
    Text(String),
    Vec3([f32; 3]),
}

impl PropValue {
    fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(v) => Some(*v),
            Self::Bool(b) => Some(i64::from(*b)),
            Self::Float(f) if f.is_finite() => Some(*f as i64),
            _ => None,
        }
    }

    fn as_float(&self) -> Option<f32> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(v) => Some(*v as f32),
            _ => None,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            Self::Int(v) => Some(*v != 0),
            _ => None,
        }
    }

    fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(t) => Some(t),
            _ => None,
        }
    }
}

/// A list the game makes at run time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    /// `[story_ids]` of `game.ltx`: names with numbers.
    StoryIds,
    /// `[spawn_story_ids]`.
    SpawnStoryIds,
    /// `[location_<n>]`: names with numbers.
    Locations(u8),
    /// The level names of `[levels]`.
    Levels,
    /// Every character profile.
    CharacterProfiles,
    /// The keys of `smart_covers.descriptions`.
    SmartCovers,
}

/// What a choose dialog lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChooseKind {
    /// Choosing a visual.
    Visual,
    /// The motions of the model `param` names.
    SkeletonAnims,
    /// The bones of the model `param` names.
    SkeletonBones,
    /// Choosing a game animation.
    GameAnim,
    /// Choosing a light animation.
    LightAnim,
    /// Choosing a texture, under `folder`.
    Texture,
    /// Choosing a sound source.
    SoundSource,
    /// The sections `param` lists.
    SpawnItem,
}

/// How a property is edited, with the widget's limits.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PropKind {
    U8 {
        min: u8,
        max: u8,
        step: u8,
    },
    U16 {
        min: u16,
        max: u16,
        step: u16,
    },
    U32 {
        min: u32,
        max: u32,
        step: u32,
    },
    Float {
        min: f32,
        max: f32,
        step: f32,
        decimals: u8,
    },
    /// Radians, shown in degrees.
    Angle {
        min: f32,
        max: f32,
    },
    /// A `BOOL`.
    Bool,
    /// One bit of a flags word; `off` / `on` name the two states when the game names them.
    Flag {
        size: u8,
        bit: u32,
        off: Option<&'static str>,
        on: Option<&'static str>,
    },
    /// A fixed token list.
    Token {
        size: u8,
        items: &'static [(&'static str, u32)],
    },
    /// A number picked by name from a run-time list.
    ListToken {
        size: u8,
        list: ListKind,
    },
    /// A name picked from a run-time list.
    List {
        list: ListKind,
    },
    Choose {
        choose: ChooseKind,
        param: Option<String>,
        max_items: u32,
        folder: Option<&'static str>,
    },
    Text,
    /// `0xAARRGGBB`.
    Color,
    /// The game types, one bit each.
    GameType {
        bits: &'static [(&'static str, u32)],
    },
    Vec3,
}

/// One row of the property sheet.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Prop {
    pub path: String,
    #[serde(flatten)]
    pub kind: PropKind,
    pub value: PropValue,
    pub read_only: bool,
    #[serde(skip)]
    pub(crate) field: Field,
}

impl Prop {
    /// The last segment of the path.
    #[must_use]
    pub fn label(&self) -> &str {
        self.path.rsplit('\\').next().unwrap_or(&self.path)
    }

    /// The path without its last segment.
    #[must_use]
    pub fn group(&self) -> &str {
        self.path.rfind('\\').map_or("", |i| &self.path[..i])
    }
}

/// A smart cover description as `smart_covers.descriptions` has it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoverDescription {
    /// `is_combat_cover`; `None` when the table has no such field (the game takes true).
    pub is_combat_cover: Option<bool>,
    pub loopholes: Vec<SmolStr>,
}

/// What the game's scripts know that the sheet needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PropContext {
    pub smart_covers: BTreeMap<SmolStr, CoverDescription>,
    /// `fp_data.story_names` / `spawn_story_names`: the `story_ids` and `spawn_story_ids`
    /// sections (id = name), an empty name for -1 (no story id) added by the reader.
    pub story_names: BTreeMap<u32, SmolStr>,
    pub spawn_story_names: BTreeMap<u32, SmolStr>,
    /// The Spawn tool's name of a section (its `$spawn` list path).
    pub profiles: BTreeMap<SmolStr, SmolStr>,
    /// `fp_data.locations`: a graph point's four location lists (`[location_N]` of `game.ltx`,
    /// name = the value as written, id = `atoi` of the key), in file order.
    pub locations: [Vec<(SmolStr, i64)>; 4],
}

impl PropContext {
    /// The token lists of the story ids from a game's `story_ids` / `spawn_story_ids` lines.
    #[must_use]
    pub fn with_story_names(
        mut self,
        story: impl IntoIterator<Item = (String, String)>,
        spawn: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        // `atoi(N)`: a key that is no number is story id 0
        let read = |lines: &mut dyn Iterator<Item = (String, String)>, none: &str| {
            let mut m: BTreeMap<u32, SmolStr> = lines
                .map(|(k, v)| (k.trim().parse::<i64>().unwrap_or(0) as u32, SmolStr::new(v.trim())))
                .collect();
            m.insert(u32::MAX, SmolStr::new(none));
            m
        };
        self.story_names = read(&mut story.into_iter(), "");
        self.spawn_story_names = read(&mut spawn.into_iter(), "");
        self
    }

    /// The location lists from the lines of `[location_0]` .. `[location_3]`.
    #[must_use]
    pub fn with_locations(mut self, lists: [Vec<(String, String)>; 4]) -> Self {
        self.locations = lists.map(|lines| {
            lines.into_iter().map(|(k, v)| (SmolStr::new(v), k.trim().parse::<i64>().unwrap_or(0))).collect()
        });
        self
    }
}

impl PropContext {
    /// Whether a smart cover description is a combat cover.
    fn is_combat_cover(&self, description: &str) -> bool {
        if description.is_empty() {
            return false;
        }
        self.smart_covers.get(description).is_none_or(|d| d.is_combat_cover.unwrap_or(true))
    }
}

/// What a change asks the editor to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SetEffect {
    /// Reload the model.
    pub visual_changed: bool,
    /// Restart the animation.
    pub animation_changed: bool,
    /// The change affects the object's motion.
    pub motion_changed: bool,
    /// The set of properties may differ now: list them again.
    pub refresh: bool,
}

const GAME_TYPES: &[(&str, u32)] = &[
    ("single", 1 << 0),
    ("deathmatch", 1 << 1),
    ("team deathmatch", 1 << 2),
    ("artefact hunt", 1 << 3),
    ("capture the artefact", 1 << 4),
    ("domination zone", 1 << 5),
    ("team domination zone", 1 << 6),
];
const RESTRICTOR_TYPES: &[(&str, u32)] = &[
    ("NOT A restrictor", 3),
    ("NONE default restrictor", 0),
    ("OUT default restrictor", 1),
    ("IN default restrictor", 2),
];
const PO_TYPES: &[(&str, u32)] = &[("Box", 0), ("Fixed chain", 1), ("Free chain", 2), ("Skeleton", 3)];

/// The field a property edits.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Field {
    #[default]
    None,
    CustomData,
    ScriptStory,
    AlifeFlag(u32),
    StoryId,
    SpawnStoryId,
    Visual,
    Animation,
    VisualFlag(u8),
    Motion,
    GpLocation(usize),
    GpLevel,
    GpPoint,
    Money,
    TraderFlag(u32),
    Profile,
    GroupCount,
    RestrictorType,
    LcLevel,
    LcPoint,
    LcSilent,
    PhType,
    PhMass,
    SkeletonFlag(u8),
    PhFixedBones,
    Lamp(LampField),
    LampFlag(u16),
    EngineSound,
    CarHealth,
    BreakableHealth,
    TeamBase,
    Condition,
    WpnAmmoType,
    WpnElapsed,
    WpnAddon(u8),
    AmmoLeft,
    AnomalyValue,
    DocInfo,
    ZoneShift,
    ZoneEnabled,
    ZoneDisabled,
    ZoneRadius,
    ZoneArtefactCount,
    AttackAnimation,
    CrTeam,
    CrSquad,
    CrGroup,
    CrHealth,
    OutRestrictions,
    InRestrictions,
    Rat(usize),
    Zombie(usize),
    CoverHold,
    CoverDescription,
    CoverEnter,
    CoverExit,
    CoverCombat,
    CoverCanFire,
    Loophole(SmolStr),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LampField {
    Color,
    Brightness,
    ColorAnimator,
    Range,
    VirtualSize,
    Texture,
    MainBone,
    ConeAngle,
    AmbientRadius,
    AmbientPower,
    AmbientTexture,
    AmbientBone,
    VolumetricQuality,
    VolumetricIntensity,
    VolumetricDistance,
    FixedBones,
    GlowRadius,
    GlowTexture,
    Health,
}

struct B<'a> {
    out: Vec<Prop>,
    s: &'a str,
}

fn float(min: f32, max: f32, step: f32) -> PropKind {
    PropKind::Float { min, max, step, decimals: 2 }
}

fn flag(size: u8, bit: u32) -> PropKind {
    PropKind::Flag { size, bit, off: None, on: None }
}

fn choose(choose: ChooseKind, param: Option<&str>, max_items: u32, folder: Option<&'static str>) -> PropKind {
    PropKind::Choose { choose, param: param.map(str::to_owned), max_items, folder }
}

fn text(v: &str) -> PropValue {
    PropValue::Text(v.to_owned())
}

impl B<'_> {
    /// Adds a property under the entity's own key: `<name>\<rest>`.
    fn add(&mut self, rest: &str, kind: PropKind, value: PropValue, field: Field) {
        self.out.push(Prop { path: format!("{}\\{rest}", self.s), kind, value, read_only: false, field });
    }

    /// A key without the section.
    fn add_bare(&mut self, path: &str, kind: PropKind, value: PropValue, field: Field) {
        self.out.push(Prop { path: path.to_owned(), kind, value, read_only: false, field });
    }

    fn alife(&mut self, _h: &Header, o: &Object) {
        // the game's property sheet has its Game Type row commented out
        let a = &o.alife;
        self.add("Custom data", PropKind::Text, text(&a.ini_string), Field::CustomData);
        // the address quests use: [story_object] in the custom data, surfaced as its own field
        self.add(
            "ALife\\Story ID (script)",
            PropKind::Text,
            text(&script_story_of(&a.ini_string)),
            Field::ScriptStory,
        );
        let bit = |f: u32| PropValue::Bool(a.flags & f != 0);
        if a.flags & alife_flags::USE_SWITCHES != 0 {
            self.add(
                "ALife\\Can switch online",
                flag(32, alife_flags::SWITCH_ONLINE),
                bit(alife_flags::SWITCH_ONLINE),
                Field::AlifeFlag(alife_flags::SWITCH_ONLINE),
            );
            self.add(
                "ALife\\Can switch offline",
                flag(32, alife_flags::SWITCH_OFFLINE),
                bit(alife_flags::SWITCH_OFFLINE),
                Field::AlifeFlag(alife_flags::SWITCH_OFFLINE),
            );
        }
        self.add(
            "ALife\\Interactive",
            flag(32, alife_flags::INTERACTIVE),
            bit(alife_flags::INTERACTIVE),
            Field::AlifeFlag(alife_flags::INTERACTIVE),
        );
        self.add(
            "ALife\\Used AI locations",
            flag(32, alife_flags::USED_AI_LOCATIONS),
            bit(alife_flags::USED_AI_LOCATIONS),
            Field::AlifeFlag(alife_flags::USED_AI_LOCATIONS),
        );
        self.add(
            "ALife\\Story ID",
            PropKind::ListToken { size: 32, list: ListKind::StoryIds },
            PropValue::Int(i64::from(a.story_id)),
            Field::StoryId,
        );
        self.add(
            "ALife\\Spawn Story ID",
            PropKind::ListToken { size: 32, list: ListKind::SpawnStoryIds },
            PropValue::Int(i64::from(a.spawn_story_id)),
            Field::SpawnStoryId,
        );
    }

    fn visual(&mut self, o: &Object) {
        let v = &o.visual;
        self.add("Model\\Visual", choose(ChooseKind::Visual, None, 1, None), text(&v.name), Field::Visual);
        self.add(
            "Model\\Animation",
            choose(ChooseKind::SkeletonAnims, Some(&v.name), 1, None),
            text(&v.startup_animation),
            Field::Animation,
        );
        self.add(
            "Model\\Obstacle",
            flag(8, u32::from(VISUAL_OBSTACLE)),
            PropValue::Bool(v.flags & VISUAL_OBSTACLE != 0),
            Field::VisualFlag(VISUAL_OBSTACLE),
        );
    }

    fn motion(&mut self, o: &Object) {
        self.add("Motion", choose(ChooseKind::GameAnim, None, 1, None), text(&o.motion), Field::Motion);
    }

    fn dyn_visual(&mut self, h: &Header, o: &Object) {
        self.alife(h, o);
        self.visual(o);
    }

    fn restrictor(&mut self, h: &Header, o: &Object) {
        self.alife(h, o);
        let t = o.restrictor().map_or(0, |r| r.restrictor_type);
        self.add(
            "restrictor type",
            PropKind::Token { size: 8, items: RESTRICTOR_TYPES },
            PropValue::Int(i64::from(t)),
            Field::RestrictorType,
        );
        let on = o.alife.flags & alife_flags::CHECK_FOR_SEPARATOR != 0;
        self.add(
            "check for separator",
            flag(32, alife_flags::CHECK_FOR_SEPARATOR),
            PropValue::Bool(on),
            Field::AlifeFlag(alife_flags::CHECK_FOR_SEPARATOR),
        );
    }

    fn trader(&mut self, o: &Object) {
        let Some(t) = o.trader() else { return };
        self.add(
            "Money",
            PropKind::U32 { min: 0, max: u32::MAX, step: 1 },
            PropValue::Int(i64::from(t.money)),
            Field::Money,
        );
        self.add(
            "Trader\\Infinite ammo",
            flag(32, TRADER_INFINITE_AMMO),
            PropValue::Bool(t.flags & TRADER_INFINITE_AMMO != 0),
            Field::TraderFlag(TRADER_INFINITE_AMMO),
        );
        self.add(
            "npc profile",
            PropKind::List { list: ListKind::CharacterProfiles },
            text(&t.character_profile),
            Field::Profile,
        );
    }

    fn inventory_item(&mut self, o: &Object) {
        let condition = o.item().map_or(1.0, |i| i.condition);
        self.add("Item condition", float(0.0, 1.0, 0.01), PropValue::Float(condition), Field::Condition);
        let bit = |f: u32| PropValue::Bool(o.alife.flags & f != 0);
        self.add(
            "ALife\\Useful for AI",
            flag(32, alife_flags::USEFUL_FOR_AI),
            bit(alife_flags::USEFUL_FOR_AI),
            Field::AlifeFlag(alife_flags::USEFUL_FOR_AI),
        );
        self.add(
            "ALife\\Visible for AI",
            flag(32, alife_flags::VISIBLE_FOR_AI),
            bit(alife_flags::VISIBLE_FOR_AI),
            Field::AlifeFlag(alife_flags::VISIBLE_FOR_AI),
        );
    }

    fn item(&mut self, h: &Header, o: &Object) {
        self.dyn_visual(h, o);
        self.inventory_item(o);
    }

    fn custom_zone(&mut self, h: &Header, o: &Object) {
        self.restrictor(h, o);
        let Some(z) = o.zone() else { return };
        let u = |v: u32| PropValue::Int(i64::from(v));
        let k = PropKind::U32 { min: 0, max: 100_000, step: 1 };
        self.add("on/off mode\\Shift time (sec)", k.clone(), u(z.start_time_shift), Field::ZoneShift);
        self.add("on/off mode\\Enabled time (sec)", k.clone(), u(z.enabled_time), Field::ZoneEnabled);
        self.add("on/off mode\\Disabled time (sec)", k, u(z.disabled_time), Field::ZoneDisabled);
    }

    fn anomalous_zone(&mut self, h: &Header, o: &Object) {
        self.custom_zone(h, o);
        let Some(z) = o.zone() else { return };
        self.add(
            "offline interactive radius",
            float(0.0, 100.0, 0.01),
            PropValue::Float(z.offline_interactive_radius),
            Field::ZoneRadius,
        );
        self.add(
            "ALife\\Artefact spawn places count",
            PropKind::U16 { min: 32, max: 256, step: 1 },
            PropValue::Int(i64::from(z.artefact_spawn_count)),
            Field::ZoneArtefactCount,
        );
        let on = o.alife.flags & alife_flags::VISIBLE_FOR_AI != 0;
        self.add(
            "ALife\\Visible for AI",
            flag(32, alife_flags::VISIBLE_FOR_AI),
            PropValue::Bool(on),
            Field::AlifeFlag(alife_flags::VISIBLE_FOR_AI),
        );
    }

    fn creature(&mut self, h: &Header, o: &Object) {
        self.dyn_visual(h, o);
        let Some(c) = o.creature() else { return };
        let k = PropKind::U8 { min: 0, max: 64, step: 1 };
        self.add("Team", k.clone(), PropValue::Int(i64::from(c.team)), Field::CrTeam);
        self.add("Squad", k.clone(), PropValue::Int(i64::from(c.squad)), Field::CrSquad);
        self.add("Group", k, PropValue::Int(i64::from(c.group)), Field::CrGroup);
        self.add("Personal\\Health", float(0.0, 2.0, 5.0), PropValue::Float(c.health), Field::CrHealth);
    }

    fn monster(&mut self, h: &Header, o: &Object) {
        self.creature(h, o);
        let bit = |f: u32| PropValue::Bool(o.alife.flags & f != 0);
        self.add(
            "ALife\\No move in offline",
            flag(32, alife_flags::OFFLINE_NO_MOVE),
            bit(alife_flags::OFFLINE_NO_MOVE),
            Field::AlifeFlag(alife_flags::OFFLINE_NO_MOVE),
        );
        self.add(
            "Use smart terrain tasks",
            flag(32, alife_flags::USE_SMART_TERRAINS),
            bit(alife_flags::USE_SMART_TERRAINS),
            Field::AlifeFlag(alife_flags::USE_SMART_TERRAINS),
        );
        let Some(m) = o.monster() else { return };
        if !m.space_restriction_section.is_empty() {
            let gcs = Some(m.space_restriction_section.as_str());
            self.add(
                "out space restrictions",
                choose(ChooseKind::SpawnItem, gcs, 16, None),
                text(&m.out_space_restrictors),
                Field::OutRestrictions,
            );
            self.add(
                "in space restrictions",
                choose(ChooseKind::SpawnItem, gcs, 16, None),
                text(&m.in_space_restrictors),
                Field::InRestrictions,
            );
        }
    }

    fn physic(&mut self, h: &Header, o: &Object) {
        self.dyn_visual(h, o);
        let Some(p) = o.physic() else { return };
        self.add(
            "Type",
            PropKind::Token { size: 32, items: PO_TYPES },
            PropValue::Int(i64::from(p.kind)),
            Field::PhType,
        );
        self.add("Mass", float(0.1, 10000.0, 0.01), PropValue::Float(p.mass), Field::PhMass);
        let active = o.skeleton.flags & skeleton_flags::ACTIVE != 0;
        self.add(
            "Active",
            flag(8, u32::from(skeleton_flags::ACTIVE)),
            PropValue::Bool(active),
            Field::SkeletonFlag(skeleton_flags::ACTIVE),
        );
        self.add(
            "Model\\Fixed bones",
            choose(ChooseKind::SkeletonBones, Some(&o.visual.name), 8, None),
            text(&p.fixed_bones),
            Field::PhFixedBones,
        );
    }

    fn lamp(&mut self, h: &Header, o: &Object) {
        self.dyn_visual(h, o);
        let Some(l) = o.lamp() else { return };
        let fl = |b: &mut Self, rest: &str, bit: u16, off: Option<&'static str>, on: Option<&'static str>| {
            b.add(
                rest,
                PropKind::Flag { size: 16, bit: u32::from(bit), off, on },
                PropValue::Bool(l.flags & bit != 0),
                Field::LampFlag(bit),
            );
        };
        let f = |v: f32| PropValue::Float(v);
        let model = Some(o.visual.name.as_str());
        fl(self, "Flags\\Physic", lamp_flags::PHYSIC, None, None);
        fl(self, "Flags\\Cast Shadow", lamp_flags::CAST_SHADOW, None, None);
        fl(self, "Flags\\Allow R1", lamp_flags::R1, None, None);
        fl(self, "Flags\\Allow R2", lamp_flags::R2, None, None);
        fl(self, "Flags\\Allow Ambient", lamp_flags::POINT_AMBIENT, None, None);
        fl(self, "Light\\Type", lamp_flags::TYPE_SPOT, Some("Point"), Some("Spot"));
        self.add(
            "Light\\Main\\Color",
            PropKind::Color,
            PropValue::Int(i64::from(l.color)),
            Field::Lamp(LampField::Color),
        );
        self.add(
            "Light\\Main\\Brightness",
            float(0.1, 5.0, 0.01),
            f(l.brightness),
            Field::Lamp(LampField::Brightness),
        );
        self.add(
            "Light\\Main\\Color Animator",
            choose(ChooseKind::LightAnim, None, 1, None),
            text(&l.color_animator),
            Field::Lamp(LampField::ColorAnimator),
        );
        self.add("Light\\Main\\Range", float(0.1, 1000.0, 0.01), f(l.range), Field::Lamp(LampField::Range));
        self.add(
            "Light\\Main\\Virtual Size",
            float(0.0, 100.0, 0.01),
            f(l.virtual_size),
            Field::Lamp(LampField::VirtualSize),
        );
        self.add(
            "Light\\Main\\Texture",
            choose(ChooseKind::Texture, None, 1, Some("lights")),
            text(&l.light_texture),
            Field::Lamp(LampField::Texture),
        );
        self.add(
            "Light\\Main\\Bone",
            choose(ChooseKind::SkeletonBones, model, 1, None),
            text(&l.light_main_bone),
            Field::Lamp(LampField::MainBone),
        );
        if l.flags & lamp_flags::TYPE_SPOT != 0 {
            let (min, max) = (1.0f32.to_radians(), 120.0f32.to_radians());
            self.add(
                "Light\\Main\\Cone Angle",
                PropKind::Angle { min, max },
                f(l.spot_cone_angle),
                Field::Lamp(LampField::ConeAngle),
            );
            fl(self, "Flags\\Volumetric", lamp_flags::VOLUMETRIC, None, None);
        }
        if l.flags & lamp_flags::POINT_AMBIENT != 0 {
            self.add(
                "Light\\Ambient\\Radius",
                float(0.0, 1000.0, 0.01),
                f(l.ambient_radius),
                Field::Lamp(LampField::AmbientRadius),
            );
            self.add(
                "Light\\Ambient\\Power",
                float(0.0, 1.0, 0.01),
                f(l.ambient_power),
                Field::Lamp(LampField::AmbientPower),
            );
            self.add(
                "Light\\Ambient\\Texture",
                choose(ChooseKind::Texture, None, 1, Some("lights")),
                text(&l.ambient_texture),
                Field::Lamp(LampField::AmbientTexture),
            );
            self.add(
                "Light\\Ambient\\Bone",
                choose(ChooseKind::SkeletonBones, model, 1, None),
                text(&l.light_ambient_bone),
                Field::Lamp(LampField::AmbientBone),
            );
        }
        if l.flags & lamp_flags::VOLUMETRIC != 0 {
            self.add(
                "Light\\Volumetric\\Quality",
                float(0.0, 1.0, 0.01),
                f(l.volumetric_quality),
                Field::Lamp(LampField::VolumetricQuality),
            );
            self.add(
                "Light\\Volumetric\\Intensity",
                float(0.0, 10.0, 0.01),
                f(l.volumetric_intensity),
                Field::Lamp(LampField::VolumetricIntensity),
            );
            self.add(
                "Light\\Volumetric\\Distance",
                float(0.0, 1.0, 0.01),
                f(l.volumetric_distance),
                Field::Lamp(LampField::VolumetricDistance),
            );
        }
        self.add(
            "Model\\Fixed bones",
            choose(ChooseKind::SkeletonBones, model, 8, None),
            text(&l.fixed_bones),
            Field::Lamp(LampField::FixedBones),
        );
        self.add(
            "Glow\\Radius",
            float(0.01, 100.0, 0.01),
            f(l.glow_radius),
            Field::Lamp(LampField::GlowRadius),
        );
        self.add(
            "Glow\\Texture",
            choose(ChooseKind::Texture, None, 1, Some("glow")),
            text(&l.glow_texture),
            Field::Lamp(LampField::GlowTexture),
        );
        self.add("Game\\Health", float(0.0, 100.0, 0.01), f(l.health), Field::Lamp(LampField::Health));
    }

    fn weapon(&mut self, h: &Header, o: &Object) {
        self.item(h, o);
        let Some(w) = o.weapon() else { return };
        self.add(
            "Ammo type:",
            PropKind::U8 { min: 0, max: 255, step: 1 },
            PropValue::Int(i64::from(w.ammo_type)),
            Field::WpnAmmoType,
        );
        self.add(
            "Ammo: in magazine",
            PropKind::U16 { min: 0, max: 30, step: 1 },
            PropValue::Int(i64::from(w.a_elapsed)),
            Field::WpnElapsed,
        );
        let addon = |b: &mut Self, rest: &str, bit: u8| {
            b.add(
                rest,
                flag(8, u32::from(bit)),
                PropValue::Bool(w.addon_flags & bit != 0),
                Field::WpnAddon(bit),
            );
        };
        if w.scope_status == ADDON_ATTACHABLE {
            addon(self, "Addons\\Scope", addon_flags::SCOPE);
        }
        if w.silencer_status == ADDON_ATTACHABLE {
            addon(self, "Addons\\Silencer", addon_flags::SILENCER);
        }
        if w.grenade_launcher_status == ADDON_ATTACHABLE {
            addon(self, "Addons\\Podstvolnik", addon_flags::GRENADE_LAUNCHER);
        }
    }

    fn smart_cover(&mut self, o: &Object, ctx: &PropContext) {
        let Some(c) = o.cover() else { return };
        self.add(
            "hold position time",
            float(0.0, 60.0, 0.01),
            PropValue::Float(c.hold_position_time),
            Field::CoverHold,
        );
        self.add(
            "description",
            PropKind::List { list: ListKind::SmartCovers },
            text(&c.description),
            Field::CoverDescription,
        );
        self.add(
            "enter min enemy distance",
            float(0.0, 100.0, 0.01),
            PropValue::Float(c.enter_min_enemy_distance),
            Field::CoverEnter,
        );
        self.add(
            "exit min enemy distance",
            float(0.0, 100.0, 0.01),
            PropValue::Float(c.exit_min_enemy_distance),
            Field::CoverExit,
        );
        if ctx.is_combat_cover(&c.description) {
            self.add(
                "is combat cover",
                PropKind::Bool,
                PropValue::Bool(c.is_combat_cover != 0),
                Field::CoverCombat,
            );
            self.add("can fire", PropKind::Bool, PropValue::Bool(c.can_fire != 0), Field::CoverCanFire);
        }
        // se_smart_cover:FillProps: a switch per loophole of the description
        if let Script::SmartCover { last_description, loopholes } = &o.script {
            let Some(d) = ctx.smart_covers.get(&c.description) else { return };
            for id in &d.loopholes {
                let on = if *last_description == c.description {
                    loopholes.iter().find(|(have, _)| have == id).is_none_or(|(_, on)| *on)
                } else {
                    true
                };
                self.add(
                    &format!("loopholes\\{id}"),
                    PropKind::Bool,
                    PropValue::Bool(on),
                    Field::Loophole(id.clone()),
                );
            }
        }
    }
}

const RAT_LABELS: [(&str, f32, f32, f32); 20] = [
    ("Personal\\Field of view", 0.0, 170.0, 10.0),
    ("Personal\\Eye range", 0.0, 300.0, 10.0),
    ("Personal\\Minimum speed", 0.0, 10.0, 0.1),
    ("Personal\\Maximum speed", 0.0, 10.0, 0.1),
    ("Personal\\Attack speed", 0.0, 10.0, 0.1),
    ("Personal\\Pursuit distance", 0.0, 300.0, 10.0),
    ("Personal\\Home distance", 0.0, 300.0, 10.0),
    ("Morale\\Success attack quant", -100.0, 100.0, 5.0),
    ("Morale\\Death quant", -100.0, 100.0, 5.0),
    ("Morale\\Fear quant", -100.0, 100.0, 5.0),
    ("Morale\\Restore quant", -100.0, 100.0, 5.0),
    ("Morale\\Restore time interval", 0.0, 65535.0, 500.0),
    ("Morale\\Minimum value", -100.0, 100.0, 5.0),
    ("Morale\\Maximum value", -100.0, 100.0, 5.0),
    ("Morale\\Normal value", -100.0, 100.0, 5.0),
    ("Attack\\Hit power", 0.0, 200.0, 5.0),
    ("Attack\\Hit interval", 0.0, 65535.0, 500.0),
    ("Attack\\Distance", 0.0, 300.0, 10.0),
    ("Attack\\Maximum angle", 0.0, 180.0, 10.0),
    ("Attack\\Success probability", 0.0, 100.0, 1.0),
];

const ZOMBIE_LABELS: [(&str, f32, f32, f32); 11] = [
    ("Personal\\Field of view", 0.0, 170.0, 10.0),
    ("Personal\\Eye range", 0.0, 300.0, 10.0),
    ("Personal\\Minimum speed", 0.0, 10.0, 0.1),
    ("Personal\\Maximum speed", 0.0, 10.0, 0.1),
    ("Personal\\Attack speed", 0.0, 10.0, 0.1),
    ("Personal\\Pursuit distance", 0.0, 300.0, 10.0),
    ("Personal\\Home distance", 0.0, 300.0, 10.0),
    ("Attack\\Hit power", 0.0, 200.0, 5.0),
    ("Attack\\Hit interval", 0.0, 65535.0, 500.0),
    ("Attack\\Distance", 0.0, 300.0, 10.0),
    ("Attack\\Maximum angle", 0.0, 100.0, 1.0),
];

/// The integer fields among the rat's and the zombie's (`u16`); the rest are floats.
const RAT_U16: [usize; 2] = [11, 16];
const ZOMBIE_U16: [usize; 1] = [8];

enum Num<'a> {
    F(&'a mut f32),
    U(&'a mut u16),
}

fn rat_field(r: &mut crate::model::Rat, i: usize) -> Option<Num<'_>> {
    Some(match i {
        0 => Num::F(&mut r.eye_fov),
        1 => Num::F(&mut r.eye_range),
        2 => Num::F(&mut r.min_speed),
        3 => Num::F(&mut r.max_speed),
        4 => Num::F(&mut r.attack_speed),
        5 => Num::F(&mut r.max_pursuit_radius),
        6 => Num::F(&mut r.max_home_radius),
        7 => Num::F(&mut r.morale_success_attack_quant),
        8 => Num::F(&mut r.morale_death_quant),
        9 => Num::F(&mut r.morale_fear_quant),
        10 => Num::F(&mut r.morale_restore_quant),
        11 => Num::U(&mut r.morale_restore_time_interval),
        12 => Num::F(&mut r.morale_min_value),
        13 => Num::F(&mut r.morale_max_value),
        14 => Num::F(&mut r.morale_normal_value),
        15 => Num::F(&mut r.hit_power),
        16 => Num::U(&mut r.hit_interval),
        17 => Num::F(&mut r.attack_distance),
        18 => Num::F(&mut r.attack_angle),
        19 => Num::F(&mut r.attack_success_probability),
        _ => return None,
    })
}

fn zombie_field(z: &mut crate::model::Zombie, i: usize) -> Option<Num<'_>> {
    Some(match i {
        0 => Num::F(&mut z.eye_fov),
        1 => Num::F(&mut z.eye_range),
        2 => Num::F(&mut z.min_speed),
        3 => Num::F(&mut z.max_speed),
        4 => Num::F(&mut z.attack_speed),
        5 => Num::F(&mut z.max_pursuit_radius),
        6 => Num::F(&mut z.max_home_radius),
        7 => Num::F(&mut z.hit_power),
        8 => Num::U(&mut z.hit_interval),
        9 => Num::F(&mut z.attack_distance),
        10 => Num::F(&mut z.attack_angle),
        _ => return None,
    })
}

fn num_value(n: Num<'_>) -> PropValue {
    match n {
        Num::F(f) => PropValue::Float(*f),
        Num::U(u) => PropValue::Int(i64::from(*u)),
    }
}

fn table_props(
    b: &mut B<'_>,
    labels: &[(&str, f32, f32, f32)],
    ints: &[usize],
    mut get: impl FnMut(usize) -> Option<PropValue>,
    field: fn(usize) -> Field,
) {
    for (i, (label, min, max, step)) in labels.iter().enumerate() {
        let Some(value) = get(i) else { continue };
        let kind = if ints.contains(&i) {
            PropKind::U16 { min: *min as u16, max: *max as u16, step: *step as u16 }
        } else {
            float(*min, *max, *step)
        };
        b.add(label, kind, value, field(i));
    }
}

/// The properties of the entity's class.
pub(crate) fn list(h: &Header, info: &ClassInfo, o: &Object, ctx: &PropContext) -> Vec<Prop> {
    let mut b = B { out: Vec::with_capacity(32), s: &h.section };
    match info.class {
        // the game's property sheet lists nothing here (its Game Type row is commented out)
        Class::Spectator => {}
        Class::Temporary => {}
        Class::GraphPoint => {
            if let Some(g) = o.graph_point() {
                for (i, loc) in g.locations.iter().enumerate() {
                    b.add(
                        &format!("Location\\{}", i + 1),
                        PropKind::ListToken { size: 8, list: ListKind::Locations(i as u8) },
                        PropValue::Int(i64::from(*loc)),
                        Field::GpLocation(i),
                    );
                }
                b.add(
                    "Connection\\Level name",
                    PropKind::List { list: ListKind::Levels },
                    text(&g.connection_level_name),
                    Field::GpLevel,
                );
                b.add(
                    "Connection\\Point name",
                    PropKind::Text,
                    text(&g.connection_point_name),
                    Field::GpPoint,
                );
            }
        }
        Class::AbstractVisual => b.visual(o),
        Class::DynamicObjectVisual
        | Class::PhSkeletonObject
        | Class::Projector
        | Class::MountedWeapon
        | Class::StationaryMgun
        | Class::InventoryBox => b.dyn_visual(h, o),
        Class::SpaceRestrictor | Class::SmartZone => b.restrictor(h, o),
        Class::LevelChanger => {
            b.restrictor(h, o);
            if let Some(lc) = o.level_changer() {
                b.add(
                    "Level to change",
                    PropKind::List { list: ListKind::Levels },
                    text(&lc.level_to_change),
                    Field::LcLevel,
                );
                b.add(
                    "Level point to change",
                    PropKind::Text,
                    text(&lc.level_point_to_change),
                    Field::LcPoint,
                );
                b.add("Silent mode", PropKind::Bool, PropValue::Bool(lc.silent_mode), Field::LcSilent);
            }
        }
        Class::TeamBaseZone => {
            b.restrictor(h, o);
            let team = o.team_base().map_or(0, |t| t.team);
            b.add(
                "team",
                PropKind::U8 { min: 0, max: 16, step: 1 },
                PropValue::Int(i64::from(team)),
                Field::TeamBase,
            );
        }
        Class::ObjectPhysic => b.physic(h, o),
        Class::HangingLamp => b.lamp(h, o),
        Class::Helicopter => {
            b.dyn_visual(h, o);
            b.motion(o);
            let sound = o.helicopter().map_or("", |x| x.engine_sound.as_str());
            b.add(
                "Engine Sound",
                choose(ChooseKind::SoundSource, None, 1, None),
                text(sound),
                Field::EngineSound,
            );
        }
        Class::Car => {
            b.dyn_visual(h, o);
            b.add(
                "Health",
                float(0.0, 1.0, 0.01),
                PropValue::Float(o.car().map_or(1.0, |c| c.health)),
                Field::CarHealth,
            );
        }
        Class::Breakable => {
            b.dyn_visual(h, o);
            b.add(
                "Health",
                float(0.0, 100.0, 0.01),
                PropValue::Float(o.breakable().map_or(1.0, |c| c.health)),
                Field::BreakableHealth,
            );
        }
        Class::Climable | Class::OnlineOfflineGroup => b.alife(h, o),
        Class::Item
        | Class::ItemTorch
        | Class::ItemDetector
        | Class::ItemPda
        | Class::ItemGrenade
        | Class::ItemExplosive
        | Class::ItemBolt
        | Class::ItemCustomOutfit
        | Class::ItemHelmet => b.item(h, o),
        Class::ItemAmmo => {
            b.item(h, o);
            if let Some(a) = o.ammo() {
                let k = PropKind::U16 { min: 0, max: a.box_size, step: a.box_size };
                b.add("Ammo: left", k, PropValue::Int(i64::from(a.elapsed)), Field::AmmoLeft);
            }
        }
        Class::ItemWeapon
        | Class::ItemWeaponMagazined
        | Class::ItemWeaponMagazinedWGL
        | Class::ItemWeaponShotGun
        | Class::ItemWeaponAutoShotGun => b.weapon(h, o),
        Class::ItemArtefact => {
            b.item(h, o);
            let v = o.artefact().map_or(100.0, |a| a.anomaly_value);
            b.add("Anomaly value:", float(0.0, 200.0, 0.01), PropValue::Float(v), Field::AnomalyValue);
        }
        Class::ItemDocument => {
            b.item(h, o);
            b.add(
                "Info portion :",
                PropKind::Text,
                text(o.document().map_or("", |d| d.doc.as_str())),
                Field::DocInfo,
            );
        }
        Class::Trader => {
            b.dyn_visual(h, o);
            b.trader(o);
        }
        Class::CustomZone => b.custom_zone(h, o),
        Class::AnomalousZone => b.anomalous_zone(h, o),
        Class::TorridZone => {
            b.custom_zone(h, o);
            b.motion(o);
        }
        Class::ZoneVisual => {
            b.anomalous_zone(h, o);
            b.visual(o);
            let anim = o.zone().map_or("", |z| z.attack_animation.as_str());
            b.add(
                "Attack animation",
                choose(ChooseKind::SkeletonAnims, Some(&o.visual.name), 1, None),
                text(anim),
                Field::AttackAnimation,
            );
        }
        Class::CreatureActor => {
            b.creature(h, o);
            b.trader(o);
        }
        Class::CreatureCrow | Class::CreaturePhantom => b.creature(h, o),
        Class::MonsterRat => {
            b.monster(h, o);
            b.inventory_item(o);
            if let Some(rat) = o.rat() {
                let mut rat = rat.clone();
                table_props(
                    &mut b,
                    &RAT_LABELS,
                    &RAT_U16,
                    |i| rat_field(&mut rat, i).map(num_value),
                    Field::Rat,
                );
            }
        }
        Class::MonsterZombie => {
            b.monster(h, o);
            if let Some(z) = o.zombie() {
                let mut z = z.clone();
                table_props(
                    &mut b,
                    &ZOMBIE_LABELS,
                    &ZOMBIE_U16,
                    |i| zombie_field(&mut z, i).map(num_value),
                    Field::Zombie,
                );
            }
        }
        Class::MonsterBase | Class::PsyDogPhantom => b.monster(h, o),
        Class::MonsterBaseGroup => {
            b.monster(h, o);
            let count = o.group().map_or(1, |g| g.count);
            b.add_bare(
                "ALife\\Count",
                PropKind::U16 { min: 0, max: 0xff, step: 1 },
                PropValue::Int(i64::from(count)),
                Field::GroupCount,
            );
        }
        Class::HumanStalker => {
            b.trader(o);
            b.monster(h, o);
            let on = o.alife.flags & alife_flags::GROUP_BEHAVIOUR != 0;
            b.add(
                "Group behaviour",
                flag(32, alife_flags::GROUP_BEHAVIOUR),
                PropValue::Bool(on),
                Field::AlifeFlag(alife_flags::GROUP_BEHAVIOUR),
            );
        }
        Class::SmartCover => b.smart_cover(o, ctx),
    }
    b.out
}

/// The header of an opaque entity, read-only.
pub(crate) fn list_opaque(h: &Header, reason: OpaqueReason) -> Vec<Prop> {
    let s = &h.section;
    let ro = |path: &str, kind: PropKind, value: PropValue| Prop {
        path: format!("{s}\\{path}"),
        kind,
        value,
        read_only: true,
        field: Field::None,
    };
    let int = |v: u32| PropValue::Int(i64::from(v));
    let u16k = PropKind::U16 { min: 0, max: u16::MAX, step: 1 };
    vec![
        ro("Packet\\Kept as bytes", PropKind::Text, text(reason.text())),
        ro("Packet\\Section", PropKind::Text, text(&h.section)),
        ro("Packet\\Name", PropKind::Text, text(&h.name)),
        ro("Packet\\Position", PropKind::Vec3, PropValue::Vec3(h.position.to_array())),
        ro("Packet\\Direction", PropKind::Vec3, PropValue::Vec3(h.angle.to_array())),
        ro("Packet\\ID", u16k.clone(), int(u32::from(h.id))),
        ro("Packet\\Parent ID", u16k.clone(), int(u32::from(h.id_parent))),
        ro("Packet\\Version", u16k.clone(), int(u32::from(h.version))),
        ro("Packet\\Script version", u16k.clone(), int(u32::from(h.script_version))),
        ro("Packet\\Spawn ID", u16k, int(u32::from(h.spawn_id))),
        ro("Packet\\Game Type", PropKind::GameType { bits: GAME_TYPES }, int(u32::from(h.game_type))),
    ]
}

fn want_int(p: &Prop, v: &PropValue) -> Result<i64> {
    v.as_int().ok_or_else(|| EntityError::WrongType { path: p.path.clone(), expected: "a number" })
}

fn want_float(p: &Prop, v: &PropValue) -> Result<f32> {
    v.as_float()
        .filter(|f| f.is_finite())
        .ok_or_else(|| EntityError::WrongType { path: p.path.clone(), expected: "a number" })
}

fn want_bool(p: &Prop, v: &PropValue) -> Result<bool> {
    v.as_bool().ok_or_else(|| EntityError::WrongType { path: p.path.clone(), expected: "true or false" })
}

/// `[story_object] story_id = <x>` inside the custom data; "" when absent.
fn script_story_of(ini: &str) -> String {
    let Some(sec) = ini.find("[story_object]") else { return String::new() };
    let mut pos = sec + "[story_object]".len();
    while pos < ini.len() {
        let eol = ini.get(pos..).and_then(|r| r.find('\n')).map_or(ini.len(), |e| pos + e);
        let line = ini.get(pos..eol).unwrap_or("");
        pos = eol + 1;
        if line.contains('[') {
            break;
        }
        let (key, val) = match line.find('=') {
            Some(eq) => (line.get(..eq).unwrap_or(""), Some(line.get(eq + 1..).unwrap_or(""))),
            None => (line, None),
        };
        let key = key.trim_start_matches([' ', '\t']).trim_end_matches([' ', '\t', '\r']);
        match val {
            // the bare "[story_object] \n id" form
            None if !key.is_empty() => return key.to_owned(),
            None => {}
            Some(v) => {
                let v = v.trim_start_matches([' ', '\t']).trim_end_matches([' ', '\t', '\r']);
                if key == "story_id" && !v.is_empty() {
                    return v.to_owned();
                }
            }
        }
    }
    String::new()
}

/// The custom data with its `[story_object]` block replaced (removed for an
/// empty id), everything else as the author wrote it.
fn with_script_story(ini: &str, id: &str) -> String {
    let mut text = ini.to_owned();
    if let Some(mut sec) = text.find("[story_object]") {
        let end = text.get(sec + 1..).and_then(|r| r.find('[')).map_or(text.len(), |e| sec + 1 + e);
        text.replace_range(sec..end, "");
        while sec > 0 && matches!(text.as_bytes().get(sec - 1), Some(b'\n' | b'\r')) {
            text.remove(sec - 1);
            sec -= 1;
        }
    }
    if !id.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push_str("\r\n");
        }
        text.push_str("[story_object]\r\nstory_id = ");
        text.push_str(id);
        text.push_str("\r\n");
    }
    text
}

fn want_text(p: &Prop, v: &PropValue) -> Result<SmolStr> {
    v.as_text()
        .map(SmolStr::new)
        .ok_or_else(|| EntityError::WrongType { path: p.path.clone(), expected: "text" })
}

/// The integer a property takes, clamped to its widget's limits.
fn clamp_int(p: &Prop, v: &PropValue) -> Result<i64> {
    let n = want_int(p, v)?;
    Ok(match &p.kind {
        PropKind::U8 { min, max, .. } => n.clamp(i64::from(*min), i64::from(*max)),
        PropKind::U16 { min, max, .. } => n.clamp(i64::from(*min), i64::from(*max)),
        PropKind::U32 { min, max, .. } => n.clamp(i64::from(*min), i64::from(*max)),
        PropKind::Token { items, .. } => {
            if !items.iter().any(|(_, value)| i64::from(*value) == n) {
                return Err(EntityError::WrongType {
                    path: p.path.clone(),
                    expected: "one of the listed values",
                });
            }
            n
        }
        PropKind::ListToken { size: 8, .. } => n.clamp(0, i64::from(u8::MAX)),
        PropKind::Color | PropKind::ListToken { .. } => n.clamp(0, i64::from(u32::MAX)),
        PropKind::GameType { .. } => n.clamp(0, i64::from(u16::MAX)),
        _ => n,
    })
}

fn clamp_float(p: &Prop, v: &PropValue) -> Result<f32> {
    let f = want_float(p, v)?;
    Ok(match &p.kind {
        PropKind::Float { min, max, .. } | PropKind::Angle { min, max } => f.clamp(*min, *max),
        _ => f,
    })
}

fn set_bit<
    T: Copy + std::ops::BitOr<Output = T> + std::ops::BitAnd<Output = T> + std::ops::Not<Output = T>,
>(
    flags: &mut T,
    bit: T,
    on: bool,
) {
    *flags = if on { *flags | bit } else { *flags & !bit };
}

/// Writes `value` into the field `prop` edits.
pub(crate) fn apply(
    _h: &mut Header,
    o: &mut Object,
    prop: &Prop,
    value: &PropValue,
    ctx: &PropContext,
) -> Result<SetEffect> {
    let mut fx = SetEffect::default();
    let p = prop;
    match &p.field {
        Field::None => return Err(EntityError::ReadOnly(p.path.clone())),
        Field::CustomData => o.alife.ini_string = want_text(p, value)?,
        Field::ScriptStory => {
            // spaces inside would split the ini line and quietly change the name
            let id: String =
                want_text(p, value)?.chars().map(|c| if c == ' ' || c == '\t' { '_' } else { c }).collect();
            o.alife.ini_string = SmolStr::new(with_script_story(&o.alife.ini_string, &id));
        }
        Field::AlifeFlag(bit) => {
            set_bit(&mut o.alife.flags, *bit, want_bool(p, value)?);
            fx.refresh = true;
        }
        Field::StoryId => o.alife.story_id = clamp_int(p, value)? as u32,
        Field::SpawnStoryId => o.alife.spawn_story_id = clamp_int(p, value)? as u32,
        Field::Visual => {
            o.visual.set(&want_text(p, value)?);
            fx.visual_changed = true;
            fx.refresh = true;
        }
        Field::Animation => {
            o.visual.startup_animation = want_text(p, value)?;
            fx.animation_changed = true;
        }
        Field::VisualFlag(bit) => set_bit(&mut o.visual.flags, *bit, want_bool(p, value)?),
        Field::Motion => {
            o.motion = want_text(p, value)?;
            fx.motion_changed = true;
        }
        Field::GpLocation(i) => {
            let v = clamp_int(p, value)? as u8;
            if let Some(slot) = o.graph_point_mut().locations.get_mut(*i) {
                *slot = v;
            }
        }
        Field::GpLevel => o.graph_point_mut().connection_level_name = want_text(p, value)?,
        Field::GpPoint => o.graph_point_mut().connection_point_name = want_text(p, value)?,
        Field::Money => o.trader_mut().money = clamp_int(p, value)? as u32,
        Field::TraderFlag(bit) => set_bit(&mut o.trader_mut().flags, *bit, want_bool(p, value)?),
        Field::Profile => o.trader_mut().character_profile = want_text(p, value)?,
        Field::GroupCount => o.group_mut().count = clamp_int(p, value)? as u16,
        Field::RestrictorType => o.restrictor_mut().restrictor_type = clamp_int(p, value)? as u8,
        Field::LcLevel => o.level_changer_mut().level_to_change = want_text(p, value)?,
        Field::LcPoint => o.level_changer_mut().level_point_to_change = want_text(p, value)?,
        Field::LcSilent => o.level_changer_mut().silent_mode = want_bool(p, value)?,
        Field::PhType => o.physic_mut().kind = clamp_int(p, value)? as u32,
        Field::PhMass => o.physic_mut().mass = clamp_float(p, value)?,
        Field::SkeletonFlag(bit) => set_bit(&mut o.skeleton.flags, *bit, want_bool(p, value)?),
        Field::PhFixedBones => o.physic_mut().fixed_bones = want_text(p, value)?,
        Field::LampFlag(bit) => {
            set_bit(&mut o.lamp_mut().flags, *bit, want_bool(p, value)?);
            fx.refresh = true;
        }
        Field::Lamp(f) => {
            let l = o.lamp_mut();
            match f {
                LampField::Color => l.color = clamp_int(p, value)? as u32,
                LampField::Brightness => l.brightness = clamp_float(p, value)?,
                LampField::ColorAnimator => l.color_animator = want_text(p, value)?,
                LampField::Range => l.range = clamp_float(p, value)?,
                LampField::VirtualSize => l.virtual_size = clamp_float(p, value)?,
                LampField::Texture => l.light_texture = want_text(p, value)?,
                LampField::MainBone => l.light_main_bone = want_text(p, value)?,
                LampField::ConeAngle => l.spot_cone_angle = clamp_float(p, value)?,
                LampField::AmbientRadius => l.ambient_radius = clamp_float(p, value)?,
                LampField::AmbientPower => l.ambient_power = clamp_float(p, value)?,
                LampField::AmbientTexture => l.ambient_texture = want_text(p, value)?,
                LampField::AmbientBone => l.light_ambient_bone = want_text(p, value)?,
                LampField::VolumetricQuality => l.volumetric_quality = clamp_float(p, value)?,
                LampField::VolumetricIntensity => l.volumetric_intensity = clamp_float(p, value)?,
                LampField::VolumetricDistance => l.volumetric_distance = clamp_float(p, value)?,
                LampField::FixedBones => l.fixed_bones = want_text(p, value)?,
                LampField::GlowRadius => l.glow_radius = clamp_float(p, value)?,
                LampField::GlowTexture => l.glow_texture = want_text(p, value)?,
                LampField::Health => l.health = clamp_float(p, value)?,
            }
        }
        Field::EngineSound => o.helicopter_mut().engine_sound = want_text(p, value)?,
        Field::CarHealth => o.car_mut().health = clamp_float(p, value)?,
        Field::BreakableHealth => o.breakable_mut().health = clamp_float(p, value)?,
        Field::TeamBase => o.team_base_mut().team = clamp_int(p, value)? as u8,
        Field::Condition => o.item_mut().condition = clamp_float(p, value)?,
        Field::WpnAmmoType => o.weapon_mut().ammo_type = clamp_int(p, value)? as u8,
        Field::WpnElapsed => o.weapon_mut().a_elapsed = clamp_int(p, value)? as u16,
        Field::WpnAddon(bit) => set_bit(&mut o.weapon_mut().addon_flags, *bit, want_bool(p, value)?),
        Field::AmmoLeft => o.ammo_mut().elapsed = clamp_int(p, value)? as u16,
        Field::AnomalyValue => o.artefact_mut().anomaly_value = clamp_float(p, value)?,
        Field::DocInfo => o.document_mut().doc = want_text(p, value)?,
        Field::ZoneShift => o.zone_mut().start_time_shift = clamp_int(p, value)? as u32,
        Field::ZoneEnabled => o.zone_mut().enabled_time = clamp_int(p, value)? as u32,
        Field::ZoneDisabled => o.zone_mut().disabled_time = clamp_int(p, value)? as u32,
        Field::ZoneRadius => o.zone_mut().offline_interactive_radius = clamp_float(p, value)?,
        Field::ZoneArtefactCount => o.zone_mut().artefact_spawn_count = clamp_int(p, value)? as u16,
        Field::AttackAnimation => o.zone_mut().attack_animation = want_text(p, value)?,
        Field::CrTeam => o.creature_mut().team = clamp_int(p, value)? as u8,
        Field::CrSquad => o.creature_mut().squad = clamp_int(p, value)? as u8,
        Field::CrGroup => o.creature_mut().group = clamp_int(p, value)? as u8,
        Field::CrHealth => o.creature_mut().health = clamp_float(p, value)?,
        Field::OutRestrictions => o.monster_mut().out_space_restrictors = want_text(p, value)?,
        Field::InRestrictions => o.monster_mut().in_space_restrictors = want_text(p, value)?,
        Field::Rat(i) => {
            let (f, u) = (clamp_float(p, value)?, clamp_int(p, value).unwrap_or(0));
            match rat_field(o.rat_mut(), *i) {
                Some(Num::F(slot)) => *slot = f,
                Some(Num::U(slot)) => *slot = u as u16,
                None => {}
            }
        }
        Field::Zombie(i) => {
            let (f, u) = (clamp_float(p, value)?, clamp_int(p, value).unwrap_or(0));
            match zombie_field(o.zombie_mut(), *i) {
                Some(Num::F(slot)) => *slot = f,
                Some(Num::U(slot)) => *slot = u as u16,
                None => {}
            }
        }
        Field::CoverHold => o.cover_mut().hold_position_time = clamp_float(p, value)?,
        Field::CoverDescription => {
            let d = want_text(p, value)?;
            o.cover_mut().description = d.clone();
            let ids: Vec<&str> = ctx
                .smart_covers
                .get(&d)
                .map(|c| c.loopholes.iter().map(SmolStr::as_str).collect())
                .unwrap_or_default();
            o.script.sync_loopholes(&d, &ids);
            fx.visual_changed = true;
            fx.refresh = true;
        }
        Field::CoverEnter => o.cover_mut().enter_min_enemy_distance = clamp_float(p, value)?,
        Field::CoverExit => o.cover_mut().exit_min_enemy_distance = clamp_float(p, value)?,
        Field::CoverCombat => o.cover_mut().is_combat_cover = u8::from(want_bool(p, value)?),
        Field::CoverCanFire => o.cover_mut().can_fire = u8::from(want_bool(p, value)?),
        Field::Loophole(id) => {
            let on = want_bool(p, value)?;
            let description = o.cover().map(|c| c.description.clone()).unwrap_or_default();
            let ids: Vec<&str> = ctx
                .smart_covers
                .get(&description)
                .map(|c| c.loopholes.iter().map(SmolStr::as_str).collect())
                .unwrap_or_default();
            o.script.sync_loopholes(&description, &ids);
            if let Script::SmartCover { loopholes, .. } = &mut o.script {
                match loopholes.iter_mut().find(|(have, _)| have == id) {
                    Some(slot) => slot.1 = on,
                    None => loopholes.push((id.clone(), on)),
                }
            }
        }
    }
    Ok(fx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Entity;
    use crate::create::fixtures::settings;

    fn paths(e: &Entity, ctx: &PropContext) -> Vec<String> {
        e.properties(ctx).into_iter().map(|p| p.path).collect()
    }

    #[test]
    fn a_stalker_lists_what_fill_props_lists() {
        let s = settings();
        let e = Entity::create("stalker", &s).unwrap();
        let p = paths(&e, &PropContext::default());
        // trader part first, then the monster, the creature, the object
        assert_eq!(p[0], "stalker\\Money");
        assert_eq!(p[1], "stalker\\Trader\\Infinite ammo");
        assert_eq!(p[2], "stalker\\npc profile");
        // no Game Type row: the game's property sheet has it commented out
        assert_eq!(p[3], "stalker\\Custom data");
        assert_eq!(p[4], "stalker\\ALife\\Story ID (script)");
        assert!(p.contains(&"stalker\\ALife\\Can switch online".to_owned()));
        assert!(p.contains(&"stalker\\Model\\Visual".to_owned()));
        assert!(p.contains(&"stalker\\Personal\\Health".to_owned()));
        assert_eq!(p.last().map(String::as_str), Some("stalker\\Group behaviour"));
    }

    #[test]
    fn set_writes_the_field_and_clamps_it() {
        let s = settings();
        let ctx = PropContext::default();
        let mut e = Entity::create("pistol", &s).unwrap();
        e.set("pistol\\Item condition", &PropValue::Float(3.0), &ctx).unwrap();
        assert_eq!(e.object().unwrap().item().unwrap().condition, 1.0);
        e.set("pistol\\Ammo: in magazine", &PropValue::Int(12), &ctx).unwrap();
        assert_eq!(e.object().unwrap().weapon().unwrap().a_elapsed, 12);
        // scope_status = 2: the scope can be attached
        e.set("pistol\\Addons\\Scope", &PropValue::Bool(true), &ctx).unwrap();
        assert_eq!(e.object().unwrap().weapon().unwrap().addon_flags, addon_flags::SCOPE);
        assert!(matches!(
            e.set("pistol\\Addons\\Podstvolnik", &PropValue::Bool(true), &ctx),
            Err(EntityError::NoProperty(_))
        ));
        assert!(matches!(
            e.set("pistol\\Item condition", &PropValue::Text("x".into()), &ctx),
            Err(EntityError::WrongType { .. })
        ));
        let fx = e.set("pistol\\Model\\Visual", &PropValue::Text("Weapons\\PM.ogf".into()), &ctx).unwrap();
        assert!(fx.visual_changed);
        assert_eq!(e.visual_name(), Some("weapons\\pm"));
    }

    #[test]
    fn switching_flags_change_what_is_listed() {
        let s = settings();
        let ctx = PropContext::default();
        let mut e = Entity::create("lamp", &s).unwrap();
        assert!(paths(&e, &ctx).contains(&"lamp\\Light\\Main\\Cone Angle".to_owned()));
        assert!(!paths(&e, &ctx).contains(&"lamp\\Light\\Ambient\\Radius".to_owned()));
        e.set("lamp\\Light\\Type", &PropValue::Bool(false), &ctx).unwrap();
        e.set("lamp\\Flags\\Allow Ambient", &PropValue::Bool(true), &ctx).unwrap();
        let p = paths(&e, &ctx);
        assert!(!p.contains(&"lamp\\Light\\Main\\Cone Angle".to_owned()));
        assert!(p.contains(&"lamp\\Light\\Ambient\\Radius".to_owned()));
        let kind = e.properties(&ctx).into_iter().find(|p| p.path == "lamp\\Light\\Type").map(|p| p.kind);
        assert_eq!(kind, Some(PropKind::Flag { size: 16, bit: 16, off: Some("Point"), on: Some("Spot") }));
    }

    #[test]
    fn smart_cover_loopholes_come_from_the_description() {
        let s = settings();
        let mut ctx = PropContext::default();
        ctx.smart_covers.insert(
            "combat_prone".into(),
            CoverDescription {
                is_combat_cover: Some(false),
                loopholes: vec!["prone".into(), "stand".into()],
            },
        );
        let mut e = Entity::create("smart_cover", &s).unwrap();
        e.set("smart_cover\\description", &PropValue::Text("combat_prone".into()), &ctx).unwrap();
        let p = paths(&e, &ctx);
        assert!(p.contains(&"smart_cover\\loopholes\\prone".to_owned()));
        assert!(!p.contains(&"smart_cover\\is combat cover".to_owned()));
        e.set("smart_cover\\loopholes\\stand", &PropValue::Bool(false), &ctx).unwrap();
        let back = Entity::from_spawn_packet(&e.to_spawn_packet(), &s).unwrap();
        assert!(matches!(
            back.object().unwrap().script(),
            Script::SmartCover { loopholes, .. } if loopholes == &vec![("prone".into(), true), ("stand".into(), false)]
        ));
    }

    #[test]
    fn values_serialise_for_mcp() {
        let s = settings();
        let e = Entity::create("ammo_9x18", &s).unwrap();
        let p =
            e.properties(&PropContext::default()).into_iter().find(|p| p.label() == "Ammo: left").unwrap();
        assert_eq!(p.group(), "ammo_9x18");
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["type"], "u16");
        assert_eq!(json["max"], 20);
        assert_eq!(json["value"]["int"], 20);
    }
}

#[cfg(test)]
mod script_story_tests {
    use super::{script_story_of, with_script_story};

    #[test]
    fn the_story_object_block_is_read_and_replaced() {
        let ini = "[logic]\r\ncfg = a.ltx\r\n[story_object]\r\nstory_id = trader\r\n";
        assert_eq!(script_story_of(ini), "trader");
        assert_eq!(script_story_of("[story_object]\nbare_id\n"), "bare_id");
        assert_eq!(script_story_of("[logic]\r\n"), "");
        assert_eq!(
            with_script_story(ini, "boss"),
            "[logic]\r\ncfg = a.ltx\r\n[story_object]\r\nstory_id = boss\r\n"
        );
        assert_eq!(with_script_story(ini, ""), "[logic]\r\ncfg = a.ltx");
        assert_eq!(with_script_story("", "x"), "[story_object]\r\nstory_id = x\r\n");
    }
}
