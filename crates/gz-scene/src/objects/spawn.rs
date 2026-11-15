//! Spawn points: a game entity, a respawn point, an environment modifier or the module's Player
//! Start, with a stalker's standing order and what a radio plays.

use std::sync::Arc;

use gz_base::{Blob, SmolStr};
use gz_entities::{Entity, Settings};

use crate::math::fmt_g;
use crate::model::Object;

/// The names the Spawn tool's list gives the points that are no config section.
pub const RPOINT_CHOOSE_NAME: &str = "$rpoint";
pub const ENVMOD_CHOOSE_NAME: &str = "$env_mod";
pub const PLAYERSTART_CHOOSE_NAME: &str = "$player_start";

/// The kinds of a spawn point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointKind {
    RPoint = 0,
    EnvMod = 1,
    Spawn = 2,
    PlayerStart = 3,
}

impl PointKind {
    #[must_use]
    pub fn from_u32(v: u32) -> Option<Self> {
        Some(match v {
            0 => Self::RPoint,
            1 => Self::EnvMod,
            2 => Self::Spawn,
            3 => Self::PlayerStart,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct XmsOrder {
    /// The order kinds: none 0, guard, patrol, wander, follow.
    pub kind: u8,
    pub route: SmolStr,
    pub look: SmolStr,
    pub radius: f32,
    pub target: SmolStr,
    pub distance: f32,
    pub flags: u8,
}

pub const ORDER_RUN: u8 = 1 << 0;
pub const ORDER_HOSTILE: u8 = 1 << 1;
const ORDER_KEYS: [&str; 5] = ["", "guard", "patrol", "wander", "follow"];

impl Default for XmsOrder {
    fn default() -> Self {
        Self {
            kind: 0,
            route: SmolStr::default(),
            look: SmolStr::default(),
            radius: 20.0,
            target: SmolStr::default(),
            distance: 3.0,
            flags: 0,
        }
    }
}

impl XmsOrder {
    /// `None` for none.
    #[must_use]
    pub fn kind_key(kind: u8) -> Option<&'static str> {
        ORDER_KEYS.get(kind as usize).copied().filter(|k| !k.is_empty())
    }

    /// 0 when unknown.
    #[must_use]
    pub fn kind_from_key(key: &str) -> u8 {
        (1..5u8).find(|i| ORDER_KEYS[*i as usize].eq_ignore_ascii_case(key)).unwrap_or(0)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kind == 0 && self.flags & ORDER_HOSTILE == 0
    }

    /// `Section`: the `[xms_order]` custom data the bake writes.
    #[must_use]
    pub fn section(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut out = String::from("[xms_order]\r\n");
        if let Some(k) = Self::kind_key(self.kind) {
            out.push_str(&format!("order = {k}\r\n"));
        }
        let run = self.flags & ORDER_RUN != 0;
        match self.kind {
            1 => {
                if !self.look.is_empty() {
                    out.push_str(&format!("look = {}\r\n", self.look));
                }
                out.push_str(&format!("radius = {}\r\n", fmt_g(self.radius)));
            }
            2 => {
                out.push_str(&format!("route = {}\r\n", self.route));
                if run {
                    out.push_str("move = run\r\n");
                }
            }
            3 => {
                out.push_str(&format!("radius = {}\r\n", fmt_g(self.radius)));
                if run {
                    out.push_str("move = run\r\n");
                }
            }
            4 => {
                out.push_str(&format!(
                    "target = {}\r\n",
                    if self.target.is_empty() { "actor" } else { &self.target }
                ));
                out.push_str(&format!("distance = {}\r\n", fmt_g(self.distance)));
            }
            _ => {}
        }
        if self.flags & ORDER_HOSTILE != 0 {
            out.push_str("hostile = true\r\n");
        }
        out
    }
}

/// A respawn point's fields.
#[derive(Clone, Debug, PartialEq)]
pub struct RPoint {
    pub team: u8,
    /// The respawn point kinds: actor spawn 0, artefact spawn, item spawn 2.
    pub kind: u8,
    pub game_type: u16,
    pub profile: SmolStr,
}

impl Default for RPoint {
    fn default() -> Self {
        Self { team: 1, kind: 0, game_type: 0xffff, profile: SmolStr::default() }
    }
}

/// An environment modifier's fields.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvMod {
    pub radius: f32,
    pub power: f32,
    pub view_dist: f32,
    pub fog_color: u32,
    pub fog_density: f32,
    pub ambient_color: u32,
    pub sky_color: u32,
    pub hemi_color: u32,
    pub flags: u16,
}

impl Default for EnvMod {
    fn default() -> Self {
        Self {
            radius: 10.0,
            power: 1.0,
            view_dist: 300.0,
            fog_color: 0x0080_8080,
            fog_density: 1.0,
            ambient_color: 0,
            sky_color: 0x00ff_ffff,
            hemi_color: 0x00ff_ffff,
            flags: 0xffff,
        }
    }
}

/// The parameters of an environment modifier: view distance, fog colour, ...
pub mod env_flags {
    pub const VIEW_DIST: u16 = 1 << 0;
    pub const FOG_COLOR: u16 = 1 << 1;
    pub const FOG_DENSITY: u16 = 1 << 2;
    pub const AMBIENT_COLOR: u16 = 1 << 3;
    pub const SKY_COLOR: u16 = 1 << 4;
    pub const HEMI_COLOR: u16 = 1 << 5;
}

/// The entity a spawn point carries. Without the linked game's configs it cannot be decoded
/// and is kept as it was read.
#[derive(Clone, Debug, PartialEq)]
pub enum SpawnEntity {
    Decoded(Arc<Entity>),
    /// What a scene file says of an entity that is not decoded (the linked game's configs are
    /// not known, or they do not read it): written back as it was.
    Kept {
        section: SmolStr,
        /// The named properties (`props`).
        props: gz_luadata::Value,
        /// The packet (`raw`).
        raw: Option<Blob>,
        /// Why it is not decoded; empty while no configs were known.
        why: String,
    },
    /// The numbered values of an X-Ray SDK scene's `_spawndata` section, read without the
    /// configs.
    RawLtx {
        section: SmolStr,
        values: Vec<String>,
    },
    /// A spawn packet of a binary X-Ray SDK scene, read without the configs.
    RawPacket {
        section: SmolStr,
        packet: Blob,
    },
}

impl SpawnEntity {
    #[must_use]
    pub fn section(&self) -> &str {
        match self {
            Self::Decoded(e) => e.section(),
            Self::Kept { section, .. } | Self::RawLtx { section, .. } | Self::RawPacket { section, .. } => {
                section
            }
        }
    }

    #[must_use]
    pub fn entity(&self) -> Option<&Entity> {
        match self {
            Self::Decoded(e) => Some(e),
            _ => None,
        }
    }

    /// Decodes a kept entity once the game's configs are there.
    #[must_use]
    pub fn decode(&self, settings: &dyn Settings) -> Option<Entity> {
        match self {
            Self::Decoded(_) | Self::Kept { .. } => None,
            Self::RawLtx { values, .. } => Entity::from_scene(values, settings).ok(),
            Self::RawPacket { packet, .. } => Entity::from_spawn_packet(packet, settings).ok(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnData {
    pub kind: PointKind,
    pub entity: Option<SpawnEntity>,
    /// The spawn data flags.
    pub sd_flags: u8,
    pub order: XmsOrder,
    /// What a radio plays (`[xms_radio] playlist`) as the author wrote it: a playlist of the
    /// module by its id, `<module>.<playlist>`, `game` or `off`; empty: the player's station.
    pub plays: SmolStr,
    pub rpoint: RPoint,
    pub env: EnvMod,
    /// The shape a zone carries: out of the scene's lists.
    pub attached: Option<Arc<Object>>,
}

impl SpawnData {
    #[must_use]
    pub fn of_kind(kind: PointKind) -> Self {
        Self {
            kind,
            entity: None,
            sd_flags: 0,
            order: XmsOrder::default(),
            plays: SmolStr::default(),
            rpoint: RPoint::default(),
            env: EnvMod::default(),
            attached: None,
        }
    }

    #[must_use]
    pub fn section(&self) -> Option<&str> {
        self.entity.as_ref().map(SpawnEntity::section)
    }

    /// Whether a point can take an order: a stalker.
    #[must_use]
    pub fn can_take_order(&self) -> bool {
        self.entity
            .as_ref()
            .and_then(SpawnEntity::entity)
            .and_then(Entity::class_info)
            .is_some_and(|c| c.clsid.trim() == "AI_STL_S")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_order_is_the_custom_data_the_bake_writes() {
        let o = XmsOrder {
            kind: 2,
            route: "patrol_1".into(),
            flags: ORDER_RUN | ORDER_HOSTILE,
            ..XmsOrder::default()
        };
        assert_eq!(
            o.section(),
            "[xms_order]\r\norder = patrol\r\nroute = patrol_1\r\nmove = run\r\nhostile = true\r\n"
        );
        let g = XmsOrder { kind: 1, ..XmsOrder::default() };
        assert_eq!(g.section(), "[xms_order]\r\norder = guard\r\nradius = 20\r\n");
        assert_eq!(XmsOrder::kind_from_key("Follow"), 4);
        assert!(XmsOrder::default().is_empty());
    }
}
