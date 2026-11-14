//! The class table: which server class a `class = ...` key of a spawn section makes.
//!
//! Two sources, as in the game: the classes the engine registers itself
//! and the ones Dead Air's `class_registrator.script`
//! registers on top. A script class is a Lua class derived from an engine class; some of them
//! write fields of their own after the engine's state ([`ScriptKind`]).

use crate::settings::Settings;

/// An engine server class (`CSE_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    Spectator,
    Temporary,
    GraphPoint,
    AbstractVisual,
    DynamicObjectVisual,
    PhSkeletonObject,
    SpaceRestrictor,
    LevelChanger,
    SmartZone,
    ObjectPhysic,
    HangingLamp,
    Projector,
    Helicopter,
    Car,
    Breakable,
    Climable,
    MountedWeapon,
    StationaryMgun,
    TeamBaseZone,
    InventoryBox,
    Item,
    ItemTorch,
    ItemAmmo,
    ItemWeapon,
    ItemWeaponMagazined,
    ItemWeaponMagazinedWGL,
    ItemWeaponShotGun,
    ItemWeaponAutoShotGun,
    ItemDetector,
    ItemArtefact,
    ItemPda,
    ItemDocument,
    ItemGrenade,
    ItemExplosive,
    ItemBolt,
    ItemCustomOutfit,
    ItemHelmet,
    Trader,
    CustomZone,
    AnomalousZone,
    TorridZone,
    ZoneVisual,
    CreatureActor,
    CreatureCrow,
    CreaturePhantom,
    MonsterRat,
    MonsterZombie,
    MonsterBase,
    PsyDogPhantom,
    HumanStalker,
    OnlineOfflineGroup,
    SmartCover,
    /// `CSE_ALifeGroupTemplate<CSE_ALifeMonsterBase>`.
    MonsterBaseGroup,
}

impl Class {
    /// The engine's class name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Spectator => "CSE_Spectator",
            Self::Temporary => "CSE_Temporary",
            Self::GraphPoint => "CSE_ALifeGraphPoint",
            Self::AbstractVisual => "CSE_AbstractVisual",
            Self::DynamicObjectVisual => "CSE_ALifeDynamicObjectVisual",
            Self::PhSkeletonObject => "CSE_ALifePHSkeletonObject",
            Self::SpaceRestrictor => "CSE_ALifeSpaceRestrictor",
            Self::LevelChanger => "CSE_ALifeLevelChanger",
            Self::SmartZone => "CSE_ALifeSmartZone",
            Self::ObjectPhysic => "CSE_ALifeObjectPhysic",
            Self::HangingLamp => "CSE_ALifeObjectHangingLamp",
            Self::Projector => "CSE_ALifeObjectProjector",
            Self::Helicopter => "CSE_ALifeHelicopter",
            Self::Car => "CSE_ALifeCar",
            Self::Breakable => "CSE_ALifeObjectBreakable",
            Self::Climable => "CSE_ALifeObjectClimable",
            Self::MountedWeapon => "CSE_ALifeMountedWeapon",
            Self::StationaryMgun => "CSE_ALifeStationaryMgun",
            Self::TeamBaseZone => "CSE_ALifeTeamBaseZone",
            Self::InventoryBox => "CSE_ALifeInventoryBox",
            Self::Item => "CSE_ALifeItem",
            Self::ItemTorch => "CSE_ALifeItemTorch",
            Self::ItemAmmo => "CSE_ALifeItemAmmo",
            Self::ItemWeapon => "CSE_ALifeItemWeapon",
            Self::ItemWeaponMagazined => "CSE_ALifeItemWeaponMagazined",
            Self::ItemWeaponMagazinedWGL => "CSE_ALifeItemWeaponMagazinedWGL",
            Self::ItemWeaponShotGun => "CSE_ALifeItemWeaponShotGun",
            Self::ItemWeaponAutoShotGun => "CSE_ALifeItemWeaponAutoShotGun",
            Self::ItemDetector => "CSE_ALifeItemDetector",
            Self::ItemArtefact => "CSE_ALifeItemArtefact",
            Self::ItemPda => "CSE_ALifeItemPDA",
            Self::ItemDocument => "CSE_ALifeItemDocument",
            Self::ItemGrenade => "CSE_ALifeItemGrenade",
            Self::ItemExplosive => "CSE_ALifeItemExplosive",
            Self::ItemBolt => "CSE_ALifeItemBolt",
            Self::ItemCustomOutfit => "CSE_ALifeItemCustomOutfit",
            Self::ItemHelmet => "CSE_ALifeItemHelmet",
            Self::Trader => "CSE_ALifeTrader",
            Self::CustomZone => "CSE_ALifeCustomZone",
            Self::AnomalousZone => "CSE_ALifeAnomalousZone",
            Self::TorridZone => "CSE_ALifeTorridZone",
            Self::ZoneVisual => "CSE_ALifeZoneVisual",
            Self::CreatureActor => "CSE_ALifeCreatureActor",
            Self::CreatureCrow => "CSE_ALifeCreatureCrow",
            Self::CreaturePhantom => "CSE_ALifeCreaturePhantom",
            Self::MonsterRat => "CSE_ALifeMonsterRat",
            Self::MonsterZombie => "CSE_ALifeMonsterZombie",
            Self::MonsterBase => "CSE_ALifeMonsterBase",
            Self::PsyDogPhantom => "CSE_ALifePsyDogPhantom",
            Self::HumanStalker => "CSE_ALifeHumanStalker",
            Self::OnlineOfflineGroup => "CSE_ALifeOnlineOfflineGroup",
            Self::SmartCover => "CSE_SmartCover",
            Self::MonsterBaseGroup => "CSE_ALifeGroupTemplate<CSE_ALifeMonsterBase>",
        }
    }

    /// Derived from `CSE_ALifeObject`: has a graph vertex, flags, custom data, story ids.
    #[must_use]
    pub fn is_alife(self) -> bool {
        !matches!(self, Self::Spectator | Self::Temporary | Self::GraphPoint | Self::AbstractVisual)
    }

    /// Carries a `CSE_Visual`.
    #[must_use]
    pub fn has_visual(self) -> bool {
        !matches!(
            self,
            Self::Spectator
                | Self::Temporary
                | Self::GraphPoint
                | Self::SpaceRestrictor
                | Self::LevelChanger
                | Self::SmartZone
                | Self::Climable
                | Self::TeamBaseZone
                | Self::CustomZone
                | Self::AnomalousZone
                | Self::TorridZone
                | Self::OnlineOfflineGroup
                | Self::SmartCover
        )
    }

    /// Carries a `CSE_Shape`.
    #[must_use]
    pub fn has_shape(self) -> bool {
        matches!(
            self,
            Self::SpaceRestrictor
                | Self::LevelChanger
                | Self::SmartZone
                | Self::Climable
                | Self::TeamBaseZone
                | Self::CustomZone
                | Self::AnomalousZone
                | Self::TorridZone
                | Self::ZoneVisual
                | Self::SmartCover
        )
    }

    /// Carries a `CSE_Motion`.
    #[must_use]
    pub fn has_motion(self) -> bool {
        matches!(self, Self::Helicopter | Self::TorridZone)
    }

    /// Carries a `CSE_PHSkeleton`.
    #[must_use]
    pub fn has_skeleton(self) -> bool {
        matches!(
            self,
            Self::PhSkeletonObject
                | Self::ObjectPhysic
                | Self::HangingLamp
                | Self::Helicopter
                | Self::Car
                | Self::CreatureActor
                | Self::MonsterBase
                | Self::PsyDogPhantom
                | Self::HumanStalker
                | Self::MonsterBaseGroup
        )
    }

    /// Derived from `CSE_ALifeInventoryItem`.
    #[must_use]
    pub fn is_item(self) -> bool {
        matches!(
            self,
            Self::Item
                | Self::ItemTorch
                | Self::ItemAmmo
                | Self::ItemWeapon
                | Self::ItemWeaponMagazined
                | Self::ItemWeaponMagazinedWGL
                | Self::ItemWeaponShotGun
                | Self::ItemWeaponAutoShotGun
                | Self::ItemDetector
                | Self::ItemArtefact
                | Self::ItemPda
                | Self::ItemDocument
                | Self::ItemGrenade
                | Self::ItemExplosive
                | Self::ItemBolt
                | Self::ItemCustomOutfit
                | Self::ItemHelmet
                | Self::MonsterRat
        )
    }

    /// Derived from `CSE_ALifeItemWeapon`.
    #[must_use]
    pub fn is_weapon(self) -> bool {
        matches!(
            self,
            Self::ItemWeapon
                | Self::ItemWeaponMagazined
                | Self::ItemWeaponMagazinedWGL
                | Self::ItemWeaponShotGun
                | Self::ItemWeaponAutoShotGun
        )
    }

    /// Derived from `CSE_ALifeTraderAbstract`.
    #[must_use]
    pub fn is_trader(self) -> bool {
        matches!(self, Self::Trader | Self::CreatureActor | Self::HumanStalker)
    }

    /// Derived from `CSE_ALifeCreatureAbstract`.
    #[must_use]
    pub fn is_creature(self) -> bool {
        matches!(self, Self::CreatureActor | Self::CreatureCrow | Self::CreaturePhantom) || self.is_monster()
    }

    /// Derived from `CSE_ALifeMonsterAbstract`.
    #[must_use]
    pub fn is_monster(self) -> bool {
        matches!(
            self,
            Self::MonsterRat
                | Self::MonsterZombie
                | Self::MonsterBase
                | Self::PsyDogPhantom
                | Self::HumanStalker
                | Self::MonsterBaseGroup
        )
    }

    /// Derived from `CSE_ALifeCustomZone`.
    #[must_use]
    pub fn is_zone(self) -> bool {
        matches!(self, Self::CustomZone | Self::AnomalousZone | Self::TorridZone | Self::ZoneVisual)
    }

    /// Derived from `CSE_ALifeAnomalousZone`.
    #[must_use]
    pub fn is_anomalous_zone(self) -> bool {
        matches!(self, Self::AnomalousZone | Self::ZoneVisual)
    }

    /// Derived from `CSE_ALifeSpaceRestrictor`.
    #[must_use]
    pub fn is_restrictor(self) -> bool {
        matches!(self, Self::SpaceRestrictor | Self::LevelChanger | Self::SmartZone | Self::TeamBaseZone)
            || self.is_zone()
    }
}

/// The fields a Dead Air script class writes after the engine's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ScriptKind {
    /// An engine class, or a script class that adds nothing to the packet.
    #[default]
    None,
    /// `se_stalker`: the remembered level vertex, the active logic section, a flag.
    Stalker,
    /// `se_monster`: the remembered level vertex and the active logic section.
    Monster,
    /// `se_actor`: whether the start position was filled.
    Actor,
    /// `se_level_changer`: enabled, the hint.
    LevelChanger,
    /// `se_heli`: targets and the respawn point, when it has one.
    Heli,
    /// `se_smart_cover`: the description last seen and the loopholes that are on.
    SmartCover,
    /// `se_artefact`: the generated properties.
    Artefact,
    /// `se_eatable`: the uses left.
    Eatable,
    /// `se_smart_terrain`: arriving NPCs, jobs, the respawn state, the faction.
    SmartTerrain,
    /// `sim_squad_scripted`: targets, the respawn point, the smart terrain.
    Squad,
}

/// One row of the class table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassInfo {
    /// The class key, padded to eight characters as the game pads it.
    pub clsid: &'static str,
    pub class: Class,
    /// The Lua class (`module.class`), empty for an engine class.
    pub script_class: &'static str,
    pub script: ScriptKind,
}

/// The key cut to eight bytes and padded with spaces.
#[must_use]
pub fn clsid_key(text: &str) -> [u8; 8] {
    let mut key = [b' '; 8];
    for (slot, b) in key.iter_mut().zip(text.bytes()) {
        *slot = b;
    }
    key
}

/// The row of a class key, `None` for a class this table does not know.
#[must_use]
pub fn class_by_clsid(text: &str) -> Option<&'static ClassInfo> {
    let key = clsid_key(text);
    TABLE.iter().find(|row| row.clsid.as_bytes() == key)
}

/// The class a spawn section makes: its `class` key looked up in the table.
#[must_use]
pub fn class_of_section(section: &str, settings: &dyn Settings) -> Option<&'static ClassInfo> {
    class_by_clsid(settings.r_string(section, "class")?)
}

/// Every row: the script registrations first, since they are what the game finds for a key
/// both sides name.
#[must_use]
pub fn class_table() -> &'static [ClassInfo] {
    TABLE
}

const fn engine(clsid: &'static str, class: Class) -> ClassInfo {
    ClassInfo { clsid, class, script_class: "", script: ScriptKind::None }
}

const fn script(
    clsid: &'static str,
    class: Class,
    script_class: &'static str,
    script: ScriptKind,
) -> ClassInfo {
    ClassInfo { clsid, class, script_class, script }
}

use Class as C;
use ScriptKind as S;

#[rustfmt::skip]
static TABLE: &[ClassInfo] = &[
    // class_registrator.script of Dead Air
    script("SMRTTRRN", C::SmartZone, "smart_terrain.se_smart_terrain", S::SmartTerrain),
    script("LVL_CHNG", C::LevelChanger, "se_level_changer.se_level_changer", S::LevelChanger),
    script("S_ACTOR ", C::CreatureActor, "se_actor.se_actor", S::Actor),
    script("AI_STL_S", C::HumanStalker, "se_stalker.se_stalker", S::Stalker),
    script("C_HLCP_S", C::Helicopter, "se_heli.se_heli", S::Heli),
    script("SPC_RS_S", C::SpaceRestrictor, "se_zones.se_restrictor", S::None),
    script("O_PHYS_S", C::ObjectPhysic, "se_item.se_physic", S::None),
    script("SMRT_C_S", C::SmartCover, "se_smart_cover.se_smart_cover", S::SmartCover),
    script("O_DSTR_S", C::ObjectPhysic, "se_item.se_physic", S::None),
    script("SO_HLAMP", C::HangingLamp, "se_item.se_lamp", S::None),
    script("SCRPTCAR", C::Car, "se_car.se_car", S::None),
    script("AI_TRD_S", C::Trader, "se_stalker.se_trader", S::None),
    script("SCRPTART", C::ItemArtefact, "se_artefact.se_artefact", S::Artefact),
    script("SM_BLOOD", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_BOARW", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_DOG_S", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_FLESH", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_P_DOG", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_BURER", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_CAT_S", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_CHIMS", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_CONTR", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_IZLOM", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_POLTR", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_GIANT", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_ZOMBI", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_SNORK", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_TUSHK", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_RAT  ", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_DOG_P", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("SM_DOG_F", C::MonsterBase, "se_monster.se_monster", S::Monster),
    script("TORCH_S ", C::ItemTorch, "se_item.se_item_torch", S::None),
    script("DET_SCIE", C::ItemDetector, "se_item.se_detector", S::None),
    script("DET_ELIT", C::ItemDetector, "se_item.se_detector", S::None),
    script("DET_ADVA", C::ItemDetector, "se_item.se_detector", S::None),
    script("DET_SIMP", C::ItemDetector, "se_item.se_detector", S::None),
    script("WP_SCOPE", C::Item, "se_item.se_item", S::None),
    script("WP_SILEN", C::Item, "se_item.se_item", S::None),
    script("WP_GLAUN", C::Item, "se_item.se_item", S::None),
    script("E_STLK  ", C::ItemCustomOutfit, "se_item.se_outfit", S::None),
    script("E_HLMET ", C::ItemHelmet, "se_item.se_helmet", S::None),
    script("WP_BINOC", C::ItemWeaponMagazined, "se_item.se_weapon_magazined", S::None),
    script("WP_KNIFE", C::ItemWeapon, "se_item.se_weapon", S::None),
    script("WP_BM16 ", C::ItemWeaponShotGun, "se_item.se_weapon_shotgun", S::None),
    script("WP_GROZA", C::ItemWeaponMagazinedWGL, "se_item.se_weapon_magazined_w_gl", S::None),
    script("WP_SVD  ", C::ItemWeaponMagazined, "se_item.se_weapon_magazined", S::None),
    script("WP_AK74 ", C::ItemWeaponMagazinedWGL, "se_item.se_weapon_magazined_w_gl", S::None),
    script("WP_LR300", C::ItemWeaponMagazined, "se_item.se_weapon_magazined", S::None),
    script("WP_HPSA ", C::ItemWeaponMagazined, "se_item.se_weapon_magazined", S::None),
    script("WP_PM   ", C::ItemWeaponMagazined, "se_item.se_weapon_magazined", S::None),
    script("WP_RG6  ", C::ItemWeaponShotGun, "se_item.se_weapon_shotgun", S::None),
    script("WP_RPG7 ", C::ItemWeaponMagazined, "se_item.se_weapon_magazined", S::None),
    script("WP_SHOTG", C::ItemWeaponShotGun, "se_item.se_weapon_shotgun", S::None),
    script("WP_ASHTG", C::ItemWeaponAutoShotGun, "se_item.se_weapon_automatic_shotgun", S::None),
    script("WP_SVU  ", C::ItemWeaponMagazined, "se_item.se_weapon_magazined", S::None),
    script("WP_USP45", C::ItemWeaponMagazined, "se_item.se_weapon_magazined", S::None),
    script("WP_VAL  ", C::ItemWeaponMagazined, "se_item.se_weapon_magazined", S::None),
    script("WP_VINT ", C::ItemWeaponMagazined, "se_item.se_weapon_magazined", S::None),
    script("WP_WALTH", C::ItemWeaponMagazined, "se_item.se_weapon_magazined", S::None),
    script("ZS_BFUZZ", C::ZoneVisual, "se_zones.se_zone_visual", S::None),
    script("ZS_MBALD", C::AnomalousZone, "se_zones.se_zone_anom", S::None),
    script("ZS_GALAN", C::AnomalousZone, "se_zones.se_zone_anom", S::None),
    script("ZS_MINCE", C::AnomalousZone, "se_zones.se_zone_anom", S::None),
    script("ZS_RADIO", C::AnomalousZone, "se_zones.se_zone_anom", S::None),
    script("ZS_TORRD", C::TorridZone, "se_zones.se_zone_torrid", S::None),
    script("ZS_AMEBA", C::ZoneVisual, "se_zones.se_zone_visual", S::None),
    script("ZS_NGRAV", C::AnomalousZone, "se_zones.se_zone_anom", S::None),
    script("AMMO_S  ", C::ItemAmmo, "se_item.se_ammo", S::None),
    script("S_VOG25 ", C::ItemAmmo, "se_item.se_ammo", S::None),
    script("S_OG7B  ", C::ItemAmmo, "se_item.se_ammo", S::None),
    script("S_M209  ", C::ItemAmmo, "se_item.se_ammo", S::None),
    script("G_F1_S  ", C::ItemGrenade, "se_item.se_grenade", S::None),
    script("G_RGD5_S", C::ItemGrenade, "se_item.se_grenade", S::None),
    script("S_FOOD  ", C::Item, "se_item.se_eatable", S::Eatable),
    script("S_INVBOX", C::InventoryBox, "se_item.se_invbox", S::None),
    script("S_EXPLO ", C::ItemExplosive, "se_item.se_explosive", S::None),
    script("S_PDA   ", C::ItemPda, "se_item.se_pda", S::None),
    script("ON_OFF_S", C::OnlineOfflineGroup, "sim_squad_scripted.sim_squad_scripted", S::Squad),
    // the classes the game's object factory registers
    engine("O_ACTOR ", C::CreatureActor),
    engine("SPECT   ", C::Spectator),
    engine("AI_FLESH", C::MonsterBase),
    engine("AI_HIMER", C::MonsterBase),
    engine("AI_DOG_R", C::MonsterBase),
    engine("AI_STL  ", C::HumanStalker),
    engine("AI_BLOOD", C::MonsterBase),
    engine("AI_BOAR ", C::MonsterBase),
    engine("AI_DOG_B", C::MonsterBase),
    engine("AI_DOG_P", C::MonsterBase),
    engine("AI_DOG_F", C::PsyDogPhantom),
    engine("AI_BURER", C::MonsterBase),
    engine("AI_GIANT", C::MonsterBase),
    engine("AI_CONTR", C::MonsterBase),
    engine("AI_POLTR", C::MonsterBase),
    engine("AI_ZOM  ", C::MonsterBase),
    engine("AI_FRACT", C::MonsterBase),
    engine("AI_SNORK", C::MonsterBase),
    engine("AI_CAT  ", C::MonsterBase),
    engine("AI_TUSH ", C::MonsterBase),
    engine("AI_PHANT", C::CreaturePhantom),
    engine("AI_TRADE", C::Trader),
    engine("AI_CROW ", C::CreatureCrow),
    engine("AI_RAT  ", C::MonsterRat),
    engine("C_NIVA  ", C::Car),
    engine("C_HLCPTR", C::Helicopter),
    engine("AF_MBALL", C::ItemArtefact),
    engine("AF_BDROP", C::ItemArtefact),
    engine("AF_NEEDL", C::ItemArtefact),
    engine("AF_BAST ", C::ItemArtefact),
    engine("AF_BGRAV", C::ItemArtefact),
    engine("AF_DUMMY", C::ItemArtefact),
    engine("AF_ZUDA ", C::ItemArtefact),
    engine("AF_THORN", C::ItemArtefact),
    engine("AF_FBALL", C::ItemArtefact),
    engine("AF_EBALL", C::ItemArtefact),
    engine("AF_RHAIR", C::ItemArtefact),
    engine("AF_GALAN", C::ItemArtefact),
    engine("AF_GRAVI", C::ItemArtefact),
    engine("ARTEFACT", C::ItemArtefact),
    engine("AF_CTA  ", C::ItemArtefact),
    engine("W_WMAGAZ", C::ItemWeaponMagazined),
    engine("W_WMAGGL", C::ItemWeaponMagazinedWGL),
    engine("W_FN2000", C::ItemWeaponMagazinedWGL),
    engine("W_AK74  ", C::ItemWeaponMagazinedWGL),
    engine("W_LR300 ", C::ItemWeaponMagazined),
    engine("W_HPSA  ", C::ItemWeaponMagazined),
    engine("W_PM    ", C::ItemWeaponMagazined),
    engine("W_FORT  ", C::ItemWeaponMagazined),
    engine("W_BINOC ", C::ItemWeaponMagazined),
    engine("WP_PDA3D", C::ItemWeaponMagazined),
    engine("W_SHOTGN", C::ItemWeaponShotGun),
    engine("W_SVD   ", C::ItemWeaponMagazined),
    engine("W_SVU   ", C::ItemWeaponMagazined),
    engine("W_RPG7  ", C::ItemWeaponMagazined),
    engine("W_VAL   ", C::ItemWeaponMagazined),
    engine("W_VINT  ", C::ItemWeaponMagazined),
    engine("W_WALTHR", C::ItemWeaponMagazined),
    engine("W_USP45 ", C::ItemWeaponMagazined),
    engine("W_GROZA ", C::ItemWeaponMagazinedWGL),
    engine("W_KNIFE ", C::ItemWeapon),
    engine("W_BM16  ", C::ItemWeaponShotGun),
    engine("W_RG6   ", C::ItemWeaponShotGun),
    engine("AMMO    ", C::ItemAmmo),
    engine("A_VOG25 ", C::ItemAmmo),
    engine("A_OG7B  ", C::ItemAmmo),
    engine("A_M209  ", C::ItemAmmo),
    engine("W_SCOPE ", C::Item),
    engine("W_SILENC", C::Item),
    engine("W_GLAUNC", C::Item),
    engine("II_BOLT ", C::ItemBolt),
    engine("II_MEDKI", C::Item),
    engine("II_BANDG", C::Item),
    engine("II_ANTIR", C::Item),
    engine("II_FOOD ", C::Item),
    engine("II_BOTTL", C::Item),
    engine("II_EXPLO", C::ItemExplosive),
    engine("II_DOC  ", C::ItemDocument),
    engine("II_ATTCH", C::Item),
    engine("EQU_SCIE", C::ItemCustomOutfit),
    engine("EQU_STLK", C::ItemCustomOutfit),
    engine("EQU_MLTR", C::ItemCustomOutfit),
    engine("EQU_EXO ", C::ItemCustomOutfit),
    engine("EQ_HLMET", C::Item),
    engine("EQ_BAKPK", C::Item),
    engine("G_F1    ", C::ItemGrenade),
    engine("G_RGD5  ", C::ItemGrenade),
    engine("G_RPG7  ", C::Temporary),
    engine("G_FAKE  ", C::Temporary),
    engine("MP_PLBAG", C::Item),
    engine("Z_ZONE  ", C::CustomZone),
    engine("Z_MBALD ", C::AnomalousZone),
    engine("Z_MINCER", C::AnomalousZone),
    engine("Z_ACIDF ", C::AnomalousZone),
    engine("Z_GALANT", C::AnomalousZone),
    engine("Z_RADIO ", C::AnomalousZone),
    engine("Z_BFUZZ ", C::ZoneVisual),
    engine("Z_RUSTYH", C::ZoneVisual),
    engine("Z_DEAD  ", C::AnomalousZone),
    engine("LVLCHNGR", C::LevelChanger),
    engine("SCRIPTZN", C::SpaceRestrictor),
    engine("SMRTZONE", C::SmartZone),
    engine("Z_TEAMBS", C::TeamBaseZone),
    engine("Z_TORRID", C::TorridZone),
    engine("SPACE_RS", C::SpaceRestrictor),
    engine("Z_AMEBA ", C::ZoneVisual),
    engine("Z_NOGRAV", C::AnomalousZone),
    engine("Z_CFIRE ", C::AnomalousZone),
    engine("D_SIMDET", C::ItemDetector),
    engine("D_ADVANC", C::ItemDetector),
    engine("D_ELITE ", C::ItemDetector),
    engine("D_SCIENT", C::ItemDetector),
    engine("D_TORCH ", C::ItemTorch),
    engine("D_PDA   ", C::ItemPda),
    engine("D_FLARE ", C::Item),
    engine("O_SEARCH", C::Projector),
    engine("W_STMGUN", C::StationaryMgun),
    engine("O_HLAMP ", C::HangingLamp),
    engine("O_PHYSIC", C::ObjectPhysic),
    engine("SCRPTOBJ", C::DynamicObjectVisual),
    engine("O_BRKBL ", C::Breakable),
    engine("O_CLMBL ", C::Climable),
    engine("P_SKELET", C::PhSkeletonObject),
    engine("P_DSTRBL", C::ObjectPhysic),
    engine("O_HLDR_E", C::DynamicObjectVisual),
    engine("O_INVBOX", C::InventoryBox),
    engine("SMRTCOVR", C::SmartCover),
    engine("AI_FLE_G", C::MonsterBaseGroup),
    engine("AI_GRAPH", C::GraphPoint),
    engine("ON_OFF_G", C::OnlineOfflineGroup),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::MapSettings;

    #[test]
    fn keys_are_padded_like_text2clsid() {
        assert_eq!(&clsid_key("AI_STL"), b"AI_STL  ");
        assert_eq!(&clsid_key("SMRTTRRN"), b"SMRTTRRN");
        assert_eq!(&clsid_key("TOO_LONG_KEY"), b"TOO_LONG");
    }

    #[test]
    fn script_classes_win_and_unknown_is_none() {
        let stalker = class_by_clsid("AI_STL_S").unwrap();
        assert_eq!((stalker.class, stalker.script), (Class::HumanStalker, ScriptKind::Stalker));
        assert_eq!(class_by_clsid("AI_STL").unwrap().script, ScriptKind::None);
        assert_eq!(class_by_clsid("E_HLMET").unwrap().class, Class::ItemHelmet);
        assert_eq!(class_by_clsid("EQ_HLMET").unwrap().class, Class::Item);
        assert!(class_by_clsid("NOT_A_ID").is_none());
    }

    #[test]
    fn no_key_is_listed_twice() {
        for (i, a) in TABLE.iter().enumerate() {
            assert_eq!(a.clsid.len(), 8, "{}", a.clsid);
            assert!(TABLE.iter().skip(i + 1).all(|b| b.clsid != a.clsid), "{}", a.clsid);
        }
    }

    #[test]
    fn a_section_resolves_through_its_class_key() {
        let s = MapSettings::new().section("stalker", &[("class", "AI_STL_S")]).section("x", &[]);
        assert_eq!(class_of_section("stalker", &s).unwrap().class, Class::HumanStalker);
        assert!(class_of_section("x", &s).is_none());
        assert!(class_of_section("nope", &s).is_none());
    }
}
