//! Spawn points (`SpawnPoint.cpp`): a game entity, a respawn point, an environment modifier or
//! the module's Player Start, with a stalker's standing Order.

use std::sync::Arc;

use gz_base::{Blob, SmolStr};
use gz_chunk::{Reader, Writer};
use gz_entities::{Entity, IniValue, Settings};

use crate::SceneError;
use crate::ltx::{IniReader, IniWriter, fmt_g};
use crate::model::Object;
use crate::objects::simple::Body;

pub const SPAWNPOINT_VERSION: u32 = 0x17;
pub const XMS_ORDER_VERSION: u8 = 1;
/// The names the Spawn tool's list gives the points that are no config section.
pub const RPOINT_CHOOSE_NAME: &str = "$rpoint";
pub const ENVMOD_CHOOSE_NAME: &str = "$env_mod";
pub const PLAYERSTART_CHOOSE_NAME: &str = "$player_start";

/// `EPointType`.
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

/// `SXmsOrder`.
#[derive(Clone, Debug, PartialEq)]
pub struct XmsOrder {
    /// `okNone` 0, `okGuard`, `okPatrol`, `okWander`, `okFollow`.
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
    /// `KindKey`: `None` for none.
    #[must_use]
    pub fn kind_key(kind: u8) -> Option<&'static str> {
        ORDER_KEYS.get(kind as usize).copied().filter(|k| !k.is_empty())
    }

    /// `KindFromKey`: 0 when unknown.
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
    /// `rptActorSpawn` 0, `rptArtefactSpawn`, `rptItemSpawn` 2.
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

/// `eViewDist`, `eFogColor`, ... of an environment modifier.
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
    /// The numbered values of an LTX `_spawndata` section.
    RawLtx {
        section: SmolStr,
        values: Vec<String>,
    },
    /// A binary spawn packet.
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
            Self::RawLtx { section, .. } | Self::RawPacket { section, .. } => section,
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
            Self::Decoded(_) => None,
            Self::RawLtx { values, .. } => Entity::from_scene(values, settings).ok(),
            Self::RawPacket { packet, .. } => Entity::from_spawn_packet(packet, settings).ok(),
        }
    }
}

/// `CSpawnPoint`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnData {
    pub kind: PointKind,
    pub entity: Option<SpawnEntity>,
    /// `SSpawnData::m_flags` (`eSDTypeRespawn`).
    pub sd_flags: u8,
    pub order: XmsOrder,
    pub rpoint: RPoint,
    pub env: EnvMod,
    /// The shape a zone carries (`m_AttachedObject`): out of the scene's lists.
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
            rpoint: RPoint::default(),
            env: EnvMod::default(),
            attached: None,
        }
    }

    #[must_use]
    pub fn section(&self) -> Option<&str> {
        self.entity.as_ref().map(SpawnEntity::section)
    }

    /// `CanTakeOrder`: a stalker (`AI_STL_S`).
    #[must_use]
    pub fn can_take_order(&self) -> bool {
        self.entity
            .as_ref()
            .and_then(SpawnEntity::entity)
            .and_then(Entity::class_info)
            .is_some_and(|c| c.clsid.trim() == "AI_STL_S")
    }

    /// The `_spawndata` section (`SSpawnData::LoadLTX`).
    pub fn read_entity_ltx(
        r: &IniReader,
        s: &str,
        settings: Option<&dyn Settings>,
    ) -> Result<(SpawnEntity, u8), SceneError> {
        if !r.line_exist(s, "name") {
            return Err(SceneError::load("SPAWNPOINT: Can't load Spawn Data."));
        }
        let section: SmolStr = r.r_string(s, "name")?.into();
        let fl = r.opt_u32(s, "fl").unwrap_or(0) as u8;
        let mut values: Vec<(String, String)> = r
            .lines(s)
            .into_iter()
            .filter(|(k, _)| k.len() == 6 && k.bytes().all(|b| b.is_ascii_digit()))
            .collect();
        values.sort_by(|a, b| a.0.cmp(&b.0));
        let values: Vec<String> = values.into_iter().map(|(_, v)| v).collect();
        let entity = match settings.map(|st| Entity::from_scene(&values, st)) {
            Some(Ok(e)) => SpawnEntity::Decoded(Arc::new(e)),
            _ => SpawnEntity::RawLtx { section, values },
        };
        Ok((entity, fl))
    }

    /// The point's own keys (`CSpawnPoint::LoadLTX` without the common part and the attached
    /// object).
    pub fn read_ltx(
        r: &IniReader,
        s: &str,
        settings: Option<&dyn Settings>,
    ) -> Result<Body<Self>, SceneError> {
        if !r.line_exist(s, "version") || r.r_u32(s, "version")? < 0x14 {
            return Ok(Body::Refused("SPAWNPOINT: Unsupported version.".into()));
        }
        let version = r.r_u32(s, "version")?;
        let Some(kind) = PointKind::from_u32(r.r_u32(s, "type")?) else {
            return Ok(Body::Refused("SPAWNPOINT: Unsupported spawn version.".into()));
        };
        let mut d = Self::of_kind(kind);
        match kind {
            PointKind::Spawn => {
                let (e, fl) = Self::read_entity_ltx(r, &format!("{s}_spawndata"), settings)?;
                d.entity = Some(e);
                d.sd_flags = fl;
                if r.line_exist(s, "xms_order") {
                    let o = &mut d.order;
                    o.kind = XmsOrder::kind_from_key(&r.r_string(s, "xms_order")?);
                    o.route = r.opt_string(s, "xms_order_route").unwrap_or_default().into();
                    o.look = r.opt_string(s, "xms_order_look").unwrap_or_default().into();
                    o.target = r.opt_string(s, "xms_order_target").unwrap_or_default().into();
                    if r.line_exist(s, "xms_order_radius") {
                        o.radius = r.r_float(s, "xms_order_radius")?;
                    }
                    if r.line_exist(s, "xms_order_distance") {
                        o.distance = r.r_float(s, "xms_order_distance")?;
                    }
                    if r.line_exist(s, "xms_order_flags") {
                        o.flags = r.r_u8(s, "xms_order_flags")?;
                    }
                }
            }
            PointKind::PlayerStart => {}
            PointKind::RPoint => {
                if version >= 0x17 {
                    d.rpoint.profile = r.r_string(s, "rp_profile")?.into();
                }
                d.rpoint.team = r.r_u8(s, "team_id")?;
                d.rpoint.kind = r.r_u8(s, "rp_type")?;
                d.rpoint.game_type = if version == 0x14 {
                    match r.r_u8(s, "game_type")? {
                        0 => 0xffff,
                        1 => 1 << 1,
                        2 => 1 << 2,
                        3 => 1 << 3,
                        4 => 1 << 4,
                        _ => 0,
                    }
                } else {
                    r.r_u16(s, "game_type")?
                };
            }
            PointKind::EnvMod => {
                let e = &mut d.env;
                e.radius = r.r_float(s, "em_radius")?;
                e.power = r.r_float(s, "em_power")?;
                e.view_dist = r.r_float(s, "view_dist")?;
                e.fog_color = r.r_u32(s, "fog_color")?;
                e.fog_density = r.r_float(s, "fog_density")?;
                e.ambient_color = r.r_u32(s, "ambient_color")?;
                e.sky_color = r.r_u32(s, "sky_color")?;
                e.hemi_color = r.r_u32(s, "hemi_color")?;
                if version >= 0x16 {
                    e.flags = r.r_u16(s, "em_flags")?;
                }
            }
        }
        Ok(Body::Read(d))
    }

    /// `CSpawnPoint::SaveLTX` without the common part and the attached object.
    pub fn write_ltx(&self, w: &mut IniWriter, s: &str, name: &str) -> Result<(), SceneError> {
        w.w_u32(s, "version", SPAWNPOINT_VERSION);
        w.w_u32(s, "type", self.kind as u32);
        match self.kind {
            PointKind::Spawn => {
                let sd = format!("{s}_spawndata");
                match &self.entity {
                    Some(SpawnEntity::Decoded(e)) => {
                        let values = e.to_scene().ok_or_else(|| {
                            SceneError::Save(format!("The spawn data of '{name}' cannot be written as text."))
                        })?;
                        w.w_string(&sd, "name", e.section());
                        w.w_u8(&sd, "fl", self.sd_flags);
                        for (i, v) in values.iter().enumerate() {
                            w.w_raw(
                                &sd,
                                &IniValue::key(u32::try_from(i + 1).unwrap_or(u32::MAX)),
                                &v.to_ltx(),
                            );
                        }
                    }
                    Some(SpawnEntity::RawLtx { section, values }) => {
                        w.w_string(&sd, "name", section);
                        w.w_u8(&sd, "fl", self.sd_flags);
                        for (i, v) in values.iter().enumerate() {
                            w.w_raw(&sd, &IniValue::key(u32::try_from(i + 1).unwrap_or(u32::MAX)), v);
                        }
                    }
                    Some(SpawnEntity::RawPacket { .. }) | None => {
                        return Err(SceneError::Save(format!(
                            "The spawn data of '{name}' came from a binary scene and cannot be written as text without the linked game's configs."
                        )));
                    }
                }
                if !self.order.is_empty() {
                    let o = &self.order;
                    w.w_string(s, "xms_order", XmsOrder::kind_key(o.kind).unwrap_or("none"));
                    w.w_string(s, "xms_order_route", &o.route);
                    w.w_string(s, "xms_order_look", &o.look);
                    w.w_string(s, "xms_order_target", &o.target);
                    w.w_float(s, "xms_order_radius", o.radius);
                    w.w_float(s, "xms_order_distance", o.distance);
                    w.w_u8(s, "xms_order_flags", o.flags);
                }
            }
            PointKind::PlayerStart => {}
            PointKind::RPoint => {
                w.w_u8(s, "team_id", self.rpoint.team);
                w.w_string(s, "rp_profile", &self.rpoint.profile);
                w.w_u8(s, "rp_type", self.rpoint.kind);
                w.w_u16(s, "game_type", self.rpoint.game_type);
            }
            PointKind::EnvMod => {
                let e = &self.env;
                w.w_float(s, "em_radius", e.radius);
                w.w_float(s, "em_power", e.power);
                w.w_float(s, "view_dist", e.view_dist);
                w.w_u32(s, "fog_color", e.fog_color);
                w.w_float(s, "fog_density", e.fog_density);
                w.w_u32(s, "ambient_color", e.ambient_color);
                w.w_u32(s, "sky_color", e.sky_color);
                w.w_u32(s, "hemi_color", e.hemi_color);
                w.w_u16(s, "em_flags", e.flags);
            }
        }
        Ok(())
    }

    /// `CSpawnPoint::LoadStream` without the common part and the attached object.
    pub fn read_stream(r: &Reader<'_>, settings: Option<&dyn Settings>) -> Result<Body<Self>, SceneError> {
        let version = match r.find_chunk(0xE411) {
            Some((b, false)) if b.len() == 2 => u16::from_le_bytes([b[0], b[1]]),
            _ => return Ok(Body::Refused("SPAWNPOINT: no version".into())),
        };
        if version < 0x14 {
            return Ok(Body::Refused("SPAWNPOINT: Unsupported version.".into()));
        }
        if let Some(c) = r.open_chunk(0xE419)? {
            let bad = || SceneError::load("SPAWNPOINT: Can't load Spawn Data.");
            let refb = c.bytes();
            if refb.is_empty() || refb.len() > 64 || refb.last() != Some(&0) {
                return Err(bad());
            }
            let section: SmolStr = c.reader().r_stringz()?.into();
            let mut d = Self::of_kind(PointKind::Spawn);
            if let Some(f) = r.open_chunk(0xE418)? {
                if f.bytes().len() != 1 {
                    return Err(bad());
                }
                d.sd_flags = f.reader().r_u8()?;
            }
            let sd = r.open_chunk(0xE420)?.ok_or_else(bad)?;
            let mut q = sd.reader();
            let n = q.r_u32()? as usize;
            if n >= 16384 || n > q.len() {
                return Err(bad());
            }
            let packet = Blob::from(q.take(n)?);
            d.entity = Some(match settings.map(|st| Entity::from_spawn_packet(&packet, st)) {
                Some(Ok(e)) => SpawnEntity::Decoded(Arc::new(e)),
                _ => SpawnEntity::RawPacket { section, packet },
            });
            if let Some(c) = r.open_chunk(0xE430)? {
                let mut q = c.reader();
                if q.r_u8()? == XMS_ORDER_VERSION {
                    let o = &mut d.order;
                    o.kind = q.r_u8()?;
                    o.route = q.r_stringz()?.into();
                    o.look = q.r_stringz()?.into();
                    o.radius = q.r_f32()?;
                    o.target = q.r_stringz()?.into();
                    o.distance = q.r_f32()?;
                    o.flags = q.r_u8()?;
                    if o.kind >= 5 {
                        o.kind = 0;
                    }
                }
            }
            return Ok(Body::Read(d));
        }
        let kind = match r.find_chunk(0xE417) {
            Some((b, false)) if b.len() == 4 => {
                PointKind::from_u32(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            _ => return Ok(Body::Refused("SPAWNPOINT: no type".into())),
        };
        let Some(kind) = kind else {
            return Ok(Body::Refused("SPAWNPOINT: Unsupported spawn version.".into()));
        };
        let mut d = Self::of_kind(kind);
        match kind {
            PointKind::RPoint => {
                let c =
                    r.open_chunk(0xE413)?.ok_or_else(|| SceneError::load("SPAWNPOINT: no respawn data"))?;
                let mut q = c.reader();
                d.rpoint.team = q.r_u8()?;
                d.rpoint.kind = q.r_u8()?;
                d.rpoint.game_type = q.r_u16()?;
                if version >= 0x17 {
                    d.rpoint.profile = q.r_stringz()?.into();
                }
            }
            PointKind::EnvMod => {
                let c = r
                    .open_chunk(0xE422)?
                    .ok_or_else(|| SceneError::load("SPAWNPOINT: no environment data"))?;
                if c.bytes().len() != 28 {
                    return Err(SceneError::load("SPAWNPOINT: bad environment data"));
                }
                let mut q = c.reader();
                let e = &mut d.env;
                e.radius = q.r_f32()?;
                e.power = q.r_f32()?;
                e.view_dist = q.r_f32()?;
                e.fog_color = q.r_u32()?;
                e.fog_density = q.r_f32()?;
                e.ambient_color = q.r_u32()?;
                e.sky_color = q.r_u32()?;
                if let Some(c) = r.open_chunk(0xE423)? {
                    e.hemi_color = c.reader().r_u32()?;
                }
                if let Some(c) = r.open_chunk(0xE424)? {
                    e.flags = c.reader().r_u16()?;
                }
            }
            PointKind::PlayerStart => {}
            PointKind::Spawn => return Ok(Body::Refused("SPAWNPOINT: a spawn without its entity".into())),
        }
        Ok(Body::Read(d))
    }

    /// `CSpawnPoint::SaveStream` after the version chunk and the attached object.
    pub fn write_stream(&self, w: &mut Writer, name: &str) -> Result<(), SceneError> {
        if let Some(e) = &self.entity {
            let packet: Vec<u8> = match e {
                SpawnEntity::Decoded(e) => e.to_spawn_packet(),
                SpawnEntity::RawPacket { packet, .. } => packet.to_vec(),
                SpawnEntity::RawLtx { .. } => {
                    return Err(SceneError::Save(format!(
                        "The spawn data of '{name}' came from a text scene and cannot be written as a packet without the linked game's configs."
                    )));
                }
            };
            w.chunk(0xE419, |w| w.w_stringz(e.section()));
            w.chunk(0xE418, |w| w.w_u8(self.sd_flags));
            w.chunk(0xE420, |w| {
                w.w_u32(u32::try_from(packet.len()).unwrap_or(0));
                w.w(&packet);
            });
            if !self.order.is_empty() {
                let o = &self.order;
                w.chunk(0xE430, |w| {
                    w.w_u8(XMS_ORDER_VERSION);
                    w.w_u8(o.kind);
                    w.w_stringz(&o.route);
                    w.w_stringz(&o.look);
                    w.w_f32(o.radius);
                    w.w_stringz(&o.target);
                    w.w_f32(o.distance);
                    w.w_u8(o.flags);
                });
            }
            return Ok(());
        }
        w.w_chunk(0xE417, &(self.kind as u32).to_le_bytes());
        match self.kind {
            PointKind::RPoint => w.chunk(0xE413, |w| {
                w.w_u8(self.rpoint.team);
                w.w_u8(self.rpoint.kind);
                w.w_u16(self.rpoint.game_type);
                w.w_stringz(&self.rpoint.profile);
            }),
            PointKind::EnvMod => {
                let e = &self.env;
                w.chunk(0xE422, |w| {
                    w.w_f32(e.radius);
                    w.w_f32(e.power);
                    w.w_f32(e.view_dist);
                    w.w_u32(e.fog_color);
                    w.w_f32(e.fog_density);
                    w.w_u32(e.ambient_color);
                    w.w_u32(e.sky_color);
                });
                w.chunk(0xE423, |w| w.w_u32(e.hemi_color));
                w.chunk(0xE424, |w| w.w_u16(e.flags));
            }
            PointKind::PlayerStart | PointKind::Spawn => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_section_matches_the_cpp() {
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

    #[test]
    fn raw_entities_round_trip() {
        let mut d = SpawnData::of_kind(PointKind::Spawn);
        d.entity =
            Some(SpawnEntity::RawLtx { section: "stalker".into(), values: vec!["1".into(), "\"x\"".into()] });
        let mut w = IniWriter::new();
        d.write_ltx(&mut w, "object_0", "s").unwrap();
        let r = IniReader::parse(&w.save(), "t").unwrap();
        let Body::Read(back) = SpawnData::read_ltx(&r, "object_0", None).unwrap() else { panic!() };
        assert_eq!(back.entity, d.entity);

        let mut p = SpawnData::of_kind(PointKind::Spawn);
        p.entity =
            Some(SpawnEntity::RawPacket { section: "zone".into(), packet: Blob::from(&[1u8, 2, 3][..]) });
        let mut w = Writer::new();
        w.chunk(0xE411, |w| w.w_u16(0x17));
        p.write_stream(&mut w, "z").unwrap();
        let bytes = w.into_bytes();
        let Body::Read(back) = SpawnData::read_stream(&Reader::new(&bytes), None).unwrap() else { panic!() };
        assert_eq!(back.entity, p.entity);
    }
}
