//! The data of a server entity, one struct per C++ base class.
//!
//! An [`Object`] holds the parts its class is made of; which ones is decided by the class, and
//! the constructor in `create.rs` makes exactly those. Defaults here are what the C++
//! constructors set without looking at the section.

use glam::Vec3;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::script::Script;

/// `CSE_ALifeObject::m_flags`.
pub mod alife_flags {
    pub const USE_SWITCHES: u32 = 1 << 0;
    pub const SWITCH_ONLINE: u32 = 1 << 1;
    pub const SWITCH_OFFLINE: u32 = 1 << 2;
    pub const INTERACTIVE: u32 = 1 << 3;
    pub const VISIBLE_FOR_AI: u32 = 1 << 4;
    pub const USEFUL_FOR_AI: u32 = 1 << 5;
    pub const OFFLINE_NO_MOVE: u32 = 1 << 6;
    pub const USED_AI_LOCATIONS: u32 = 1 << 7;
    pub const GROUP_BEHAVIOUR: u32 = 1 << 8;
    pub const CAN_SAVE: u32 = 1 << 9;
    pub const VISIBLE_FOR_MAP: u32 = 1 << 10;
    pub const USE_SMART_TERRAINS: u32 = 1 << 11;
    pub const CHECK_FOR_SEPARATOR: u32 = 1 << 12;
}

/// `CSE_PHSkeleton::_flags`.
pub mod skeleton_flags {
    pub const ACTIVE: u8 = 1 << 0;
    pub const SPAWN_COPY: u8 = 1 << 1;
    pub const SAVED_DATA: u8 = 1 << 2;
    pub const NOT_SAVE: u8 = 1 << 3;
}

/// `CSE_ALifeObjectHangingLamp::flags`.
pub mod lamp_flags {
    pub const PHYSIC: u16 = 1 << 0;
    pub const CAST_SHADOW: u16 = 1 << 1;
    pub const R1: u16 = 1 << 2;
    pub const R2: u16 = 1 << 3;
    pub const TYPE_SPOT: u16 = 1 << 4;
    pub const POINT_AMBIENT: u16 = 1 << 5;
    pub const VOLUMETRIC: u16 = 1 << 6;
}

/// `CSE_ALifeItemWeapon::EWeaponAddonState`.
pub mod addon_flags {
    pub const SCOPE: u8 = 0x01;
    pub const GRENADE_LAUNCHER: u8 = 0x02;
    pub const SILENCER: u8 = 0x04;
}

/// `ISE_Visual::flObstacle`.
pub const VISUAL_OBSTACLE: u8 = 1 << 0;
/// `CSE_ALifeTraderAbstract::eTraderFlagInfiniteAmmo`.
pub const TRADER_INFINITE_AMMO: u32 = 1 << 0;
/// `ALife::eAddonAttachable`.
pub const ADDON_ATTACHABLE: i32 = 2;
/// `NO_RANK` and `NO_REPUTATION`: `-type_max(int)`.
pub const NO_RANK: i32 = -i32::MAX;
/// `NO_COMMUNITY_INDEX`.
pub const NO_COMMUNITY_INDEX: i32 = -1;

/// One shape of a `CSE_Shape`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Sphere {
        center: Vec3,
        radius: f32,
    },
    /// The rows `i`, `j`, `k`, `c` of the box matrix.
    Box {
        i: Vec3,
        j: Vec3,
        k: Vec3,
        c: Vec3,
    },
    /// A type byte the engine writes no data for.
    Other(u8),
}

pub type Shapes = SmallVec<[Shape; 1]>;

/// A `SmallVec` read and written as a list.
mod small_vec {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use smallvec::{Array, SmallVec};

    pub fn serialize<A, S>(v: &SmallVec<A>, s: S) -> Result<S::Ok, S::Error>
    where
        A: Array,
        A::Item: Serialize,
        S: Serializer,
    {
        v.as_slice().serialize(s)
    }

    pub fn deserialize<'de, A, D>(d: D) -> Result<SmallVec<A>, D::Error>
    where
        A: Array,
        A::Item: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Ok(SmallVec::from_vec(Vec::<A::Item>::deserialize(d)?))
    }
}

/// `CSE_ALifeObject`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ALife {
    pub graph_id: u16,
    pub distance: f32,
    pub direct_control: bool,
    pub node_id: u32,
    #[serde(rename = "alife_flags")]
    pub flags: u32,
    /// `m_ini_string`: the custom data.
    #[serde(rename = "custom_data")]
    pub ini_string: SmolStr,
    pub story_id: u32,
    pub spawn_story_id: u32,
}

impl Default for ALife {
    fn default() -> Self {
        Self {
            graph_id: u16::MAX,
            distance: 0.0,
            direct_control: true,
            node_id: u32::MAX,
            flags: !alife_flags::OFFLINE_NO_MOVE,
            ini_string: SmolStr::default(),
            story_id: u32::MAX,
            spawn_story_id: u32::MAX,
        }
    }
}

/// `CSE_Visual`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Visual {
    #[serde(rename = "visual")]
    pub name: SmolStr,
    pub startup_animation: SmolStr,
    #[serde(rename = "visual_flags")]
    pub flags: u8,
}

impl Default for Visual {
    fn default() -> Self {
        Self { name: SmolStr::default(), startup_animation: SmolStr::new_static("$editor"), flags: 0 }
    }
}

impl Visual {
    /// `set_visual`: the name without its extension, lower case.
    pub(crate) fn set(&mut self, name: &str) {
        self.name = visual_name(name);
    }
}

/// What `set_visual` makes of a name: cut at the last dot, ASCII lower case.
#[must_use]
pub fn visual_name(name: &str) -> SmolStr {
    let cut = name.rfind('.').map_or(name, |dot| &name[..dot]);
    SmolStr::new(cut.to_ascii_lowercase())
}

/// One bone of saved physics data. Position and rotation stay in the 8-bit form the packet
/// holds, so they are written back as they were read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedBone {
    pub position: [u8; 3],
    pub rotation: [u8; 4],
    pub enabled: bool,
}

/// `SPHBonesData`: the bones a physics shell was saved with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedBones {
    pub bones_mask: u64,
    pub root_bone: u16,
    pub min: Vec3,
    pub max: Vec3,
    pub bones: Vec<SavedBone>,
}

impl Default for SavedBones {
    fn default() -> Self {
        Self {
            bones_mask: u64::MAX,
            root_bone: 0,
            min: Vec3::splat(-100.0),
            max: Vec3::splat(100.0),
            bones: Vec::new(),
        }
    }
}

impl SavedBones {
    /// The position of a bone, decoded as `r_float_q8` decodes it.
    #[must_use]
    pub fn bone_position(&self, bone: &SavedBone) -> Vec3 {
        let q = |v: u8, min: f32, max: f32| crate::packet::dequantize_q8(v, min, max).clamp(min, max);
        Vec3::new(
            q(bone.position[0], self.min.x, self.max.x),
            q(bone.position[1], self.min.y, self.max.y),
            q(bone.position[2], self.min.z, self.max.z),
        )
    }
}

/// `CSE_PHSkeleton`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhSkeleton {
    pub flags: u8,
    pub source_id: u16,
    pub saved: Option<Box<SavedBones>>,
}

impl Default for PhSkeleton {
    fn default() -> Self {
        Self { flags: 0, source_id: u16::MAX, saved: None }
    }
}

/// `CSE_ALifeGroupAbstract`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Group {
    pub create_spawn_positions: bool,
    pub count: u16,
    pub members: Vec<u16>,
}

impl Default for Group {
    fn default() -> Self {
        Self { create_spawn_positions: true, count: 1, members: Vec::new() }
    }
}

/// The physics state an item or a physic object sends in its update (`SPHNetState`), with the
/// item count it is sent under.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncState {
    pub num_items: u8,
    pub enabled: bool,
    pub force: Vec3,
    pub torque: Vec3,
    pub position: Vec3,
    pub quaternion: [f32; 4],
    pub angular_vel: Vec3,
    pub linear_vel: Vec3,
}

impl Default for SyncState {
    fn default() -> Self {
        Self {
            num_items: 0,
            enabled: false,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
            position: Vec3::ZERO,
            quaternion: [0.0, 0.0, 1.0, 0.0],
            angular_vel: Vec3::ZERO,
            linear_vel: Vec3::ZERO,
        }
    }
}

/// `CSE_ALifeInventoryItem`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InventoryItem {
    pub condition: f32,
    pub upgrades: Vec<SmolStr>,
    /// `None` is `m_u8NumItems == 0`: nothing to synchronise.
    pub sync: Option<Box<SyncState>>,
    pub mass: f32,
    pub cost: u32,
}

impl Default for InventoryItem {
    fn default() -> Self {
        Self { condition: 1.0, upgrades: Vec::new(), sync: None, mass: 0.0, cost: 0 }
    }
}

/// `CSE_ALifeTraderAbstract`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TraderAbstract {
    pub money: u32,
    pub specific_character: SmolStr,
    pub flags: u32,
    pub character_profile: SmolStr,
    pub community_index: i32,
    pub rank: i32,
    pub reputation: i32,
    pub character_name: SmolStr,
    pub deadbody_can_take: bool,
    pub deadbody_closed: bool,
    pub max_item_mass: f32,
}

impl Default for TraderAbstract {
    fn default() -> Self {
        Self {
            money: 0,
            specific_character: SmolStr::default(),
            flags: 0,
            character_profile: SmolStr::new_static("default"),
            community_index: NO_COMMUNITY_INDEX,
            rank: NO_RANK,
            reputation: NO_RANK,
            character_name: SmolStr::default(),
            deadbody_can_take: true,
            deadbody_closed: false,
            max_item_mass: 0.0,
        }
    }
}

/// `CSE_ALifeCreatureAbstract`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Creature {
    pub team: u8,
    pub squad: u8,
    pub group: u8,
    pub health: f32,
    pub out_restrictions: Vec<u16>,
    pub in_restrictions: Vec<u16>,
    pub killer_id: u16,
    pub game_death_time: u64,
    // update only
    pub timestamp: u32,
    pub flags: u8,
    pub o_model: f32,
    pub torso_yaw: f32,
    pub torso_pitch: f32,
    pub torso_roll: f32,
}

impl Default for Creature {
    fn default() -> Self {
        Self {
            team: 0,
            squad: 0,
            group: 0,
            health: 1.0,
            out_restrictions: Vec::new(),
            in_restrictions: Vec::new(),
            killer_id: u16::MAX,
            game_death_time: 0,
            timestamp: 0,
            flags: 0,
            o_model: 0.0,
            torso_yaw: 0.0,
            torso_pitch: 0.0,
            torso_roll: 0.0,
        }
    }
}

/// `CSE_ALifeMonsterAbstract`, with the one field `CSE_ALifeMonsterBase` adds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Monster {
    pub out_space_restrictors: SmolStr,
    pub in_space_restrictors: SmolStr,
    pub smart_terrain_id: u16,
    /// `m_task_reached`, the byte as the packet holds it.
    pub task_reached: u8,
    pub spec_object_id: u16,
    // update only
    pub next_graph_id: u16,
    pub prev_graph_id: u16,
    pub distance_from_point: f32,
    pub distance_to_point: f32,
    /// `SpaceRestrictionSection` of the spawn section: what the restrictor choosers list.
    pub space_restriction_section: SmolStr,
}

impl Default for Monster {
    fn default() -> Self {
        Self {
            out_space_restrictors: SmolStr::default(),
            in_space_restrictors: SmolStr::default(),
            smart_terrain_id: u16::MAX,
            task_reached: 0,
            spec_object_id: u16::MAX,
            next_graph_id: u16::MAX,
            prev_graph_id: u16::MAX,
            distance_from_point: 0.0,
            distance_to_point: 0.0,
            space_restriction_section: SmolStr::default(),
        }
    }
}

/// `CSE_ALifeHumanAbstract` and `CSE_ALifeHumanStalker`: the brain's preferences and the start
/// dialog.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Human {
    #[serde(with = "small_vec")]
    pub equipment_preferences: SmallVec<[u8; 5]>,
    #[serde(with = "small_vec")]
    pub weapon_preferences: SmallVec<[u8; 4]>,
    pub start_dialog: SmolStr,
}

impl Default for Human {
    fn default() -> Self {
        // oracle: the brain fills both with `Random.randI(3)`; a new entity gets zeros here.
        Self {
            equipment_preferences: SmallVec::from_slice(&[0; 5]),
            weapon_preferences: SmallVec::from_slice(&[0; 4]),
            start_dialog: SmolStr::default(),
        }
    }
}

/// `CSE_ALifeGraphPoint`.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphPoint {
    pub connection_level_name: SmolStr,
    pub connection_point_name: SmolStr,
    pub locations: [u8; 4],
}

/// `CSE_Temporary`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Temporary {
    pub node_id: u32,
}

impl Default for Temporary {
    fn default() -> Self {
        Self { node_id: u32::MAX }
    }
}

/// `CSE_ALifeSpaceRestrictor`.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Restrictor {
    /// `RestrictionSpace::ERestrictorTypes`.
    pub restrictor_type: u8,
}

/// `CSE_ALifeLevelChanger`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LevelChanger {
    pub next_graph_id: u16,
    pub next_node_id: u32,
    pub next_position: Vec3,
    pub angles: Vec3,
    pub level_to_change: SmolStr,
    pub level_point_to_change: SmolStr,
    pub silent_mode: bool,
}

impl Default for LevelChanger {
    fn default() -> Self {
        Self {
            next_graph_id: u16::MAX,
            next_node_id: u32::MAX,
            next_position: Vec3::ZERO,
            angles: Vec3::ZERO,
            level_to_change: SmolStr::default(),
            level_point_to_change: SmolStr::default(),
            silent_mode: false,
        }
    }
}

/// `CSE_ALifeObjectPhysic`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Physic {
    /// `EPOType`.
    pub kind: u32,
    pub mass: f32,
    pub fixed_bones: SmolStr,
    pub sync: Option<Box<SyncState>>,
}

impl Default for Physic {
    fn default() -> Self {
        Self { kind: 3, mass: 10.0, fixed_bones: SmolStr::default(), sync: None }
    }
}

/// `CSE_ALifeObjectHangingLamp`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lamp {
    pub flags: u16,
    pub color: u32,
    pub brightness: f32,
    pub color_animator: SmolStr,
    pub light_texture: SmolStr,
    pub range: f32,
    pub virtual_size: f32,
    pub light_ambient_bone: SmolStr,
    pub light_main_bone: SmolStr,
    pub fixed_bones: SmolStr,
    pub spot_cone_angle: f32,
    pub ambient_radius: f32,
    pub ambient_power: f32,
    pub ambient_texture: SmolStr,
    pub volumetric_quality: f32,
    pub volumetric_intensity: f32,
    pub volumetric_distance: f32,
    pub glow_texture: SmolStr,
    pub glow_radius: f32,
    pub health: f32,
}

impl Default for Lamp {
    fn default() -> Self {
        Self {
            flags: lamp_flags::TYPE_SPOT | lamp_flags::R1 | lamp_flags::R2,
            color: 0xffff_ffff,
            brightness: 1.0,
            color_animator: SmolStr::default(),
            light_texture: SmolStr::default(),
            range: 10.0,
            virtual_size: 0.1,
            light_ambient_bone: SmolStr::default(),
            light_main_bone: SmolStr::default(),
            fixed_bones: SmolStr::default(),
            // deg2rad(120.f)
            spot_cone_angle: 120.0 * std::f32::consts::PI / 180.0,
            ambient_radius: 10.0,
            ambient_power: 0.1,
            ambient_texture: SmolStr::default(),
            volumetric_quality: 1.0,
            volumetric_intensity: 1.0,
            volumetric_distance: 1.0,
            glow_texture: SmolStr::default(),
            glow_radius: 0.7,
            health: 100.0,
        }
    }
}

/// `CSE_ALifeHelicopter`.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Helicopter {
    pub engine_sound: SmolStr,
}

/// `CSE_ALifeCar`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Car {
    pub health: f32,
    /// `SDoorState`: open state and health.
    pub doors: Vec<(u8, f32)>,
    /// `SWheelState`: health.
    pub wheels: Vec<f32>,
}

impl Default for Car {
    fn default() -> Self {
        Self { health: 1.0, doors: Vec::new(), wheels: Vec::new() }
    }
}

/// `CSE_ALifeObjectBreakable`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Breakable {
    pub health: f32,
}

impl Default for Breakable {
    fn default() -> Self {
        Self { health: 1.0 }
    }
}

/// `CSE_ALifeObjectClimable`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Climable {
    pub material: SmolStr,
}

impl Default for Climable {
    fn default() -> Self {
        Self { material: SmolStr::new_static("materials\\fake_ladders") }
    }
}

/// `CSE_ALifeStationaryMgun`: update only.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StationaryMgun {
    pub working: bool,
    pub dest_enemy_dir: Vec3,
}

/// `CSE_ALifeTeamBaseZone`.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamBase {
    pub team: u8,
}

/// `CSE_ALifeInventoryBox`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InventoryBox {
    pub can_take: bool,
    pub closed: bool,
    pub tip_text: SmolStr,
}

impl Default for InventoryBox {
    fn default() -> Self {
        Self { can_take: true, closed: false, tip_text: SmolStr::new_static("inventory_box_use") }
    }
}

/// `CSE_ALifeItemTorch`: update only.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Torch {
    pub active: bool,
    pub nightvision_active: bool,
    pub attached: bool,
}

/// `CSE_ALifeItemAmmo`.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Ammo {
    pub elapsed: u16,
    pub box_size: u16,
}

/// `CSE_ALifeItemWeapon` and the classes derived from it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Weapon {
    pub a_current: u16,
    pub a_elapsed: u16,
    pub state: u8,
    pub addon_flags: u8,
    pub ammo_type: u8,
    /// `grenade_count_t` packed: the type in the two high bits, the count below.
    pub elapsed_grenades: u8,
    /// The game's `m_condition_type`. `None` for a state the editor's classes wrote, which
    /// ends before it; the game accepts both.
    pub condition_type: Option<u32>,
    // update only
    pub wpn_flags: u8,
    pub zoom: u8,
    pub cur_fire_mode: u8,
    pub grenade_mode: bool,
    pub ammo_ids: Vec<u8>,
    // from the section
    pub scope_status: i32,
    pub silencer_status: i32,
    pub grenade_launcher_status: i32,
    pub mag_size: u32,
}

impl Default for Weapon {
    fn default() -> Self {
        Self {
            a_current: 90,
            a_elapsed: 0,
            state: 0,
            addon_flags: 0,
            ammo_type: 0,
            elapsed_grenades: 0,
            condition_type: Some(0),
            wpn_flags: 0,
            zoom: 0,
            cur_fire_mode: 0,
            grenade_mode: false,
            ammo_ids: Vec::new(),
            scope_status: 0,
            silencer_status: 0,
            grenade_launcher_status: 0,
            mag_size: 0,
        }
    }
}

/// `CSE_ALifeItemArtefact`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Artefact {
    pub anomaly_value: f32,
}

impl Default for Artefact {
    fn default() -> Self {
        Self { anomaly_value: 100.0 }
    }
}

/// `CSE_ALifeItemPDA`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pda {
    pub original_owner: u16,
    pub specific_character: SmolStr,
    pub info_portion: SmolStr,
}

impl Default for Pda {
    fn default() -> Self {
        Self {
            original_owner: u16::MAX,
            specific_character: SmolStr::default(),
            info_portion: SmolStr::default(),
        }
    }
}

/// `CSE_ALifeItemDocument`.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Document {
    pub doc: SmolStr,
}

/// `CSE_ALifeCustomZone`, `CSE_ALifeAnomalousZone` and `CSE_ALifeZoneVisual`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Zone {
    /// The game's `m_maxPower`; the editor's classes write 0 here.
    pub max_power: f32,
    pub owner_id: u32,
    pub enabled_time: u32,
    pub disabled_time: u32,
    pub start_time_shift: u32,
    pub offline_interactive_radius: f32,
    pub artefact_spawn_count: u16,
    pub artefact_position_offset: u32,
    pub attack_animation: SmolStr,
}

impl Default for Zone {
    fn default() -> Self {
        Self {
            max_power: 1.0,
            owner_id: u32::MAX,
            enabled_time: 0,
            disabled_time: 0,
            start_time_shift: 0,
            offline_interactive_radius: 30.0,
            artefact_spawn_count: 32,
            artefact_position_offset: 0,
            attack_animation: SmolStr::default(),
        }
    }
}

/// `m_AliveState` of the actor's update.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AliveState {
    pub enabled: u8,
    pub angular_vel: Vec3,
    pub linear_vel: Vec3,
    pub force: Vec3,
    pub torque: Vec3,
    pub position: Vec3,
    pub quaternion: [f32; 4],
}

/// `CSE_ALifeCreatureActor`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Actor {
    pub holder_id: u16,
    // update only
    pub mstate: u16,
    pub accel: Vec3,
    pub velocity: Vec3,
    pub radiation: f32,
    pub weapon: u8,
    pub num_items: u16,
    pub alive: Option<Box<AliveState>>,
    pub bone_data_size: u8,
    pub dead_body: Vec<u8>,
}

impl Default for Actor {
    fn default() -> Self {
        Self {
            holder_id: u16::MAX,
            mstate: 0,
            accel: Vec3::ZERO,
            velocity: Vec3::ZERO,
            radiation: 0.0,
            weapon: 0,
            num_items: 0,
            alive: None,
            bone_data_size: 0,
            dead_body: Vec::new(),
        }
    }
}

/// `CSE_ALifeMonsterRat`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rat {
    pub eye_fov: f32,
    pub eye_range: f32,
    pub min_speed: f32,
    pub max_speed: f32,
    pub attack_speed: f32,
    pub max_pursuit_radius: f32,
    pub max_home_radius: f32,
    pub morale_success_attack_quant: f32,
    pub morale_death_quant: f32,
    pub morale_fear_quant: f32,
    pub morale_restore_quant: f32,
    pub morale_restore_time_interval: u16,
    pub morale_min_value: f32,
    pub morale_max_value: f32,
    pub morale_normal_value: f32,
    pub hit_power: f32,
    pub hit_interval: u16,
    pub attack_distance: f32,
    pub attack_angle: f32,
    pub attack_success_probability: f32,
}

impl Default for Rat {
    fn default() -> Self {
        Self {
            eye_fov: 120.0,
            eye_range: 10.0,
            min_speed: 0.5,
            max_speed: 1.5,
            attack_speed: 4.0,
            max_pursuit_radius: 100.0,
            max_home_radius: 10.0,
            morale_success_attack_quant: 20.0,
            morale_death_quant: -10.0,
            morale_fear_quant: -20.0,
            morale_restore_quant: 10.0,
            morale_restore_time_interval: 3000,
            morale_min_value: 0.0,
            morale_max_value: 100.0,
            morale_normal_value: 66.0,
            hit_power: 10.0,
            hit_interval: 1500,
            attack_distance: 0.7,
            attack_angle: 45.0,
            attack_success_probability: 0.5,
        }
    }
}

/// `CSE_ALifeMonsterZombie`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Zombie {
    pub eye_fov: f32,
    pub eye_range: f32,
    pub min_speed: f32,
    pub max_speed: f32,
    pub attack_speed: f32,
    pub max_pursuit_radius: f32,
    pub max_home_radius: f32,
    pub hit_power: f32,
    pub hit_interval: u16,
    pub attack_distance: f32,
    pub attack_angle: f32,
}

impl Default for Zombie {
    fn default() -> Self {
        Self {
            eye_fov: 120.0,
            eye_range: 30.0,
            min_speed: 1.5,
            max_speed: 1.75,
            attack_speed: 2.0,
            max_pursuit_radius: 100.0,
            max_home_radius: 30.0,
            hit_power: 20.0,
            hit_interval: 1000,
            attack_distance: 1.0,
            attack_angle: 15.0,
        }
    }
}

/// `CSE_ALifeOnlineOfflineGroup`: member ids, sorted and unique as the engine keeps them.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Squad {
    pub members: Vec<u16>,
}

/// `CSE_SmartCover`.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SmartCover {
    pub description: SmolStr,
    pub hold_position_time: f32,
    pub enter_min_enemy_distance: f32,
    pub exit_min_enemy_distance: f32,
    /// `BOOL`s the engine reads and writes as one byte.
    pub is_combat_cover: u8,
    pub can_fire: u8,
}

macro_rules! parts {
    ($( $(#[$doc:meta])* $name:ident, $name_mut:ident : $ty:ident; )*) => {
        /// The parts of a decoded entity. Which ones exist is decided by its class.
        #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct Object {
            #[serde(flatten)]
            pub(crate) alife: ALife,
            #[serde(flatten)]
            pub(crate) visual: Visual,
            pub(crate) motion: SmolStr,
            #[serde(with = "small_vec")]
            pub(crate) shapes: Shapes,
            pub(crate) skeleton: PhSkeleton,
            pub(crate) script: Script,
            $(
                #[serde(skip_serializing_if = "Option::is_none")]
                pub(crate) $name: Option<Box<$ty>>,
            )*
        }

        impl Object {
            $(
                $(#[$doc])*
                #[must_use]
                pub fn $name(&self) -> Option<&$ty> {
                    self.$name.as_deref()
                }

                pub(crate) fn $name_mut(&mut self) -> &mut $ty {
                    self.$name.get_or_insert_with(Default::default)
                }
            )*
        }
    };
}

parts! {
    /// `CSE_ALifeGroupAbstract`.
    group, group_mut: Group;
    /// `CSE_ALifeInventoryItem`.
    item, item_mut: InventoryItem;
    /// `CSE_ALifeTraderAbstract`.
    trader, trader_mut: TraderAbstract;
    /// `CSE_ALifeCreatureAbstract`.
    creature, creature_mut: Creature;
    /// `CSE_ALifeMonsterAbstract`.
    monster, monster_mut: Monster;
    /// The human brain and the stalker's start dialog.
    human, human_mut: Human;
    graph_point, graph_point_mut: GraphPoint;
    temporary, temporary_mut: Temporary;
    restrictor, restrictor_mut: Restrictor;
    level_changer, level_changer_mut: LevelChanger;
    physic, physic_mut: Physic;
    lamp, lamp_mut: Lamp;
    helicopter, helicopter_mut: Helicopter;
    car, car_mut: Car;
    breakable, breakable_mut: Breakable;
    climable, climable_mut: Climable;
    mgun, mgun_mut: StationaryMgun;
    team_base, team_base_mut: TeamBase;
    inventory_box, inventory_box_mut: InventoryBox;
    torch, torch_mut: Torch;
    ammo, ammo_mut: Ammo;
    weapon, weapon_mut: Weapon;
    artefact, artefact_mut: Artefact;
    pda, pda_mut: Pda;
    document, document_mut: Document;
    zone, zone_mut: Zone;
    actor, actor_mut: Actor;
    rat, rat_mut: Rat;
    zombie, zombie_mut: Zombie;
    squad, squad_mut: Squad;
    cover, cover_mut: SmartCover;
}

impl Object {
    /// `CSE_ALifeObject`. Meaningful for the classes derived from it.
    #[must_use]
    pub fn alife(&self) -> &ALife {
        &self.alife
    }

    /// `CSE_Visual`. Meaningful for the classes that carry one.
    #[must_use]
    pub fn visual(&self) -> &Visual {
        &self.visual
    }

    /// `CSE_Motion::motion_name`.
    #[must_use]
    pub fn motion(&self) -> &str {
        &self.motion
    }

    /// `CSE_Shape::shapes`.
    #[must_use]
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    /// `CSE_PHSkeleton`.
    #[must_use]
    pub fn skeleton(&self) -> &PhSkeleton {
        &self.skeleton
    }

    /// The fields the script class keeps after the engine's state.
    #[must_use]
    pub fn script(&self) -> &Script {
        &self.script
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_constructors() {
        let a = ALife::default();
        assert_eq!(a.flags, 0xffff_ffbf);
        assert_eq!((a.graph_id, a.node_id, a.story_id), (0xffff, u32::MAX, u32::MAX));
        assert_eq!(Lamp::default().flags, 28);
        assert!((Lamp::default().spot_cone_angle - 2.094_395_2).abs() < 1e-6);
        assert_eq!(Visual::default().startup_animation, "$editor");
        assert_eq!(TraderAbstract::default().rank, -2_147_483_647);
    }

    #[test]
    fn set_visual_cuts_the_extension_and_lowers_the_case() {
        assert_eq!(visual_name("Actors\\Stalker\\Stalker_1.ogf"), "actors\\stalker\\stalker_1");
        assert_eq!(visual_name("dynamics\\box"), "dynamics\\box");
        // strext looks for the last dot anywhere in the path
        assert_eq!(visual_name("a.b\\c"), "a");
    }

    #[test]
    fn parts_come_into_being_on_first_write() {
        let mut o = Object::default();
        assert!(o.weapon().is_none());
        o.weapon_mut().a_elapsed = 7;
        assert_eq!(o.weapon().map(|w| (w.a_current, w.a_elapsed)), Some((90, 7)));
    }
}
