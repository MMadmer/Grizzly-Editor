//! An entity: the `CSE_Abstract` header, the decoded parts of its class, and the bytes it
//! was read from. `Spawn_Write` / `Spawn_Read` of `xrServer_Object_Base.cpp`.

use std::sync::Arc;

use glam::Vec3;
use smol_str::SmolStr;

use crate::class::{Class, ClassInfo, class_of_section};
use crate::create::construct;
use crate::model::{Object, Shape, Shapes};
use crate::packet::{IniSource, IniValue, M_SPAWN, M_UPDATE, NetReader, NetWriter, TextEncoding};
use crate::props::{self, Prop, PropContext, PropValue, SetEffect};
use crate::settings::Settings;
use crate::state::{ReadCtx, state_read, state_write};
use crate::update::{update_read, update_write};
use crate::{EntityError, Result};

/// `SPAWN_VERSION` of the game.
pub const SPAWN_VERSION: u16 = 128;
/// `M_SPAWN_OBJECT_LOCAL`.
pub const M_SPAWN_OBJECT_LOCAL: u16 = 1 << 0;
/// `M_SPAWN_OBJECT_ASPLAYER`.
pub const M_SPAWN_OBJECT_ASPLAYER: u16 = 1 << 3;
/// `M_SPAWN_VERSION`: the header carries a version word.
pub const M_SPAWN_VERSION: u16 = 1 << 5;

/// The `CSE_Abstract` part every packet starts with.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    /// `s_name`: the spawn section.
    pub section: SmolStr,
    /// `s_name_replace`: the object's name.
    pub name: SmolStr,
    /// The byte after the name, read and dropped by the game (`s_gameid`); written as 0.
    pub game_id: u8,
    /// `s_RP`.
    pub rp: u8,
    pub position: Vec3,
    pub angle: Vec3,
    pub respawn_time: u16,
    pub id: u16,
    pub id_parent: u16,
    pub id_phantom: u16,
    /// `s_flags` as read.
    pub flags: u16,
    /// `m_wVersion` the packet was read with; 0 for a new entity.
    pub version: u16,
    /// `m_gameType`.
    pub game_type: u16,
    /// `m_script_version` the packet was read with.
    pub script_version: u16,
    /// `script_server_object_version()`: what a write puts into the header.
    pub write_script_version: u16,
    pub client_data: Vec<u8>,
    /// `m_tSpawnID`.
    pub spawn_id: u16,
    /// The encoding the strings came in, and are written back in.
    pub text: TextEncoding,
}

impl Header {
    fn complete(&self) -> bool {
        self.version == SPAWN_VERSION
    }
}

/// Why an entity is kept as bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpaqueReason {
    /// The section's class is not in the class table, or the section does not exist.
    UnknownClass,
    /// Written by a newer game than this crate knows.
    NewerVersion,
    /// A header without a version word (`Spawn_Read` returns false for it).
    NoVersion,
    /// The section's defaults could not be read (the constructor would stop the game).
    BadSection,
    /// The state did not decode.
    BadState,
    /// The state decoded, but did not write back to the same bytes.
    Lossy,
}

impl OpaqueReason {
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::UnknownClass => "its class is not known",
            Self::NewerVersion => "it was written by a newer game",
            Self::NoVersion => "its header has no version",
            Self::BadSection => "its section cannot be read",
            Self::BadState => "its state does not decode",
            Self::Lossy => "its state does not write back to the same bytes",
        }
    }
}

/// An entity kept as it was read.
#[derive(Clone, Debug, PartialEq)]
pub struct Opaque {
    pub header: Header,
    pub reason: OpaqueReason,
    /// What `reason` came from, for the log.
    pub detail: String,
    /// The whole spawn packet, or nothing when it came from a scene section.
    raw_spawn: Option<Arc<[u8]>>,
    /// Where the state block starts in `raw_spawn` (after its size word).
    state_at: usize,
    raw_update: Option<Arc<[u8]>>,
    /// The numbered values of a scene section, when it came from one.
    raw_ini: Option<Arc<[SmolStr]>>,
    /// The header was changed: written anew in front of the state bytes.
    header_dirty: bool,
}

impl Opaque {
    /// The bytes of the state block.
    #[must_use]
    pub fn raw_state(&self) -> &[u8] {
        self.raw_spawn.as_deref().and_then(|b| b.get(self.state_at..)).unwrap_or(&[])
    }

    /// The update packet, `M_UPDATE` word included.
    #[must_use]
    pub fn raw_update(&self) -> Option<&[u8]> {
        self.raw_update.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Known {
    header: Header,
    info: &'static ClassInfo,
    object: Object,
    /// The packets it was read from, until it is changed.
    raw_spawn: Option<Arc<[u8]>>,
    raw_update: Option<Arc<[u8]>>,
}

#[derive(Clone, Debug, PartialEq)]
enum Inner {
    Known(Box<Known>),
    Opaque(Box<Opaque>),
}

/// A server entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Entity {
    inner: Inner,
}

/// Where a level changer leads.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelChangerDestination {
    pub level: SmolStr,
    pub point: SmolStr,
    pub graph_id: u16,
    pub node_id: u32,
    pub position: Vec3,
    pub direction: Vec3,
    pub silent: bool,
}

/// The header of `Spawn_Read`, and the state size word.
fn read_header(r: &mut NetReader<'_>) -> Result<(Header, u16)> {
    let kind = r.r_begin()?;
    if kind != M_SPAWN {
        return Err(EntityError::NotSpawn(kind));
    }
    let section = r.r_stringz()?;
    let name = r.r_stringz()?;
    let game_id = r.r_u8()?;
    let rp = r.r_u8()?;
    let position = r.r_vec3()?;
    let angle = r.r_vec3()?;
    let respawn_time = r.r_u16()?;
    let id = r.r_u16()?;
    let id_parent = r.r_u16()?;
    let id_phantom = r.r_u16()?;
    let flags = r.r_u16()?;
    let version = if flags & M_SPAWN_VERSION != 0 { r.r_u16()? } else { 0 };
    let game_type = if version > 120 { r.r_u16()? } else { u16::MAX };
    let mut h = Header {
        section,
        name,
        game_id,
        rp,
        position,
        angle,
        respawn_time,
        id,
        id_parent,
        id_phantom,
        flags,
        version,
        game_type,
        script_version: 0,
        write_script_version: 0,
        client_data: Vec::new(),
        spawn_id: u16::MAX,
        text: TextEncoding::Cp1251,
    };
    if version == 0 || version > SPAWN_VERSION {
        h.text = r.text_stats().encoding();
        return Ok((h, 0));
    }
    if version > 69 {
        h.script_version = r.r_u16()?;
    }
    if version > 70 {
        let n = if version > 93 { r.r_u16()? } else { u16::from(r.r_u8()?) };
        if n > 0 {
            h.client_data = r.take(usize::from(n))?.to_vec();
        }
    }
    if version > 79 {
        h.spawn_id = r.r_u16()?;
    }
    if version < 112 {
        if version > 82 {
            r.r_float()?;
        }
        if version > 83 {
            r.r_u32()?;
            r.skip_stringz()?;
            r.r_u32()?;
            r.r_u32()?;
            r.r_u64()?;
        }
        if version > 84 {
            r.r_u64()?;
            r.r_u64()?;
        }
    }
    let size = r.r_u16()?;
    h.text = r.text_stats().encoding();
    Ok((h, size))
}

/// The header of `Spawn_Write(packet, TRUE)`; returns where the state size word is.
fn write_header(h: &Header, w: &mut NetWriter) -> usize {
    w.w_begin(M_SPAWN);
    w.w_stringz(&h.section);
    w.w_stringz(&h.name);
    w.w_u8(0);
    w.w_u8(h.rp);
    w.w_vec3(h.position);
    w.w_vec3(h.angle);
    w.w_u16(h.respawn_time);
    w.w_u16(h.id);
    w.w_u16(h.id_parent);
    w.w_u16(h.id_phantom);
    w.w_u16(h.flags | M_SPAWN_VERSION | M_SPAWN_OBJECT_LOCAL);
    w.w_u16(SPAWN_VERSION);
    w.w_u16(h.game_type);
    w.w_u16(h.write_script_version);
    w.w_u16(h.client_data.len() as u16);
    w.w(&h.client_data);
    w.w_u16(h.spawn_id);
    let at = w.w_tell();
    w.w_u16(0);
    at
}

fn close_state(w: &mut NetWriter, at: usize) {
    let size = w.w_tell().saturating_sub(at) as u16;
    w.w_seek(at, &size.to_le_bytes());
}

fn new_writer(h: &Header, ini: bool) -> NetWriter {
    if ini { NetWriter::with_ini(h.text) } else { NetWriter::new(h.text) }
}

/// What `from_any` reads from.
enum Source<'a> {
    Bin(Arc<[u8]>),
    Ini(&'a dyn IniSource),
}

impl Entity {
    /// `xrFactory_Create(section)`: a new entity with the section's defaults.
    pub fn create(section: &str, settings: &dyn Settings) -> Result<Self> {
        let info = class_of_section(section, settings)
            .ok_or_else(|| EntityError::UnknownClass(SmolStr::new(section)))?;
        // a group template is named after its monster section, as the C++ names it
        let (header, object) = construct(info, section, settings)?;
        Ok(Self {
            inner: Inner::Known(Box::new(Known { header, info, object, raw_spawn: None, raw_update: None })),
        })
    }

    /// A spawn packet (`M_SPAWN` word first), as a scene's binary chunk or `all.spawn` holds it.
    /// `Err` only when even the header cannot be read; anything after it that does not decode
    /// gives an [`Opaque`] entity.
    pub fn from_spawn_packet(bytes: &[u8], settings: &dyn Settings) -> Result<Self> {
        Self::from_any(Source::Bin(Arc::from(bytes)), settings)
    }

    /// The spawn packet and the update packet of an `all.spawn` vertex.
    pub fn from_packets(spawn: &[u8], update: &[u8], settings: &dyn Settings) -> Result<Self> {
        let mut e = Self::from_spawn_packet(spawn, settings)?;
        e.read_update(update);
        Ok(e)
    }

    /// The numbered values of an LTX scene section (`000001 = ...`).
    pub fn from_scene(values: &dyn IniSource, settings: &dyn Settings) -> Result<Self> {
        Self::from_any(Source::Ini(values), settings)
    }

    fn from_any(src: Source<'_>, settings: &dyn Settings) -> Result<Self> {
        let bytes = match &src {
            Source::Bin(b) => Some(b.clone()),
            Source::Ini(_) => None,
        };
        let mut r = match &src {
            Source::Bin(b) => NetReader::new(b),
            Source::Ini(v) => NetReader::ini(*v),
        };
        let (mut header, size) = read_header(&mut r)?;
        header.write_script_version = crate::create::current_script_version(settings);
        let state_at = r.r_tell();
        let raw_ini = match &src {
            Source::Ini(v) => Some(collect_ini(*v)),
            Source::Bin(_) => None,
        };
        let opaque = |header: Header, reason: OpaqueReason, detail: String| Self {
            inner: Inner::Opaque(Box::new(Opaque {
                header,
                reason,
                detail,
                raw_spawn: bytes.clone(),
                state_at,
                raw_update: None,
                raw_ini: raw_ini.clone(),
                header_dirty: false,
            })),
        };
        if header.version == 0 {
            return Ok(opaque(header, OpaqueReason::NoVersion, String::new()));
        }
        if header.version > SPAWN_VERSION {
            let detail = format!("version {}", header.version);
            return Ok(opaque(header, OpaqueReason::NewerVersion, detail));
        }
        let Some(info) = class_of_section(&header.section, settings) else {
            let detail = format!("section [{}]", header.section);
            return Ok(opaque(header, OpaqueReason::UnknownClass, detail));
        };
        let (made, mut object) = match construct(info, &header.section, settings) {
            Ok(v) => v,
            Err(e) => return Ok(opaque(header, OpaqueReason::BadSection, e.to_string())),
        };
        if header.version <= 79 {
            header.spawn_id = made.spawn_id;
        }
        let v = header.version;
        let read = {
            let mut ctx = ReadCtx { o: &mut object, h: &mut header, info, r: &mut r, v };
            state_read(&mut ctx)
        };
        if let Err(e) = read {
            return Ok(opaque(header, OpaqueReason::BadState, e.to_string()));
        }
        let stats = r.text_stats();
        header.text = stats.encoding();

        // A packet of today's layout must write back to its own bytes, or it is kept as bytes.
        if let Some(b) = &bytes
            && header.version == SPAWN_VERSION
            && header.script_version == header.write_script_version
        {
            let state = b.get(state_at..).unwrap_or(&[]);
            let mut w = NetWriter::new(header.text);
            state_write(&object, &header, info.class, &mut w);
            if usize::from(size) != state.len() + 2 || w.bytes() != state {
                return Ok(opaque(header, OpaqueReason::Lossy, String::new()));
            }
        }
        Ok(Self {
            inner: Inner::Known(Box::new(Known { header, info, object, raw_spawn: bytes, raw_update: None })),
        })
    }

    /// Reads an update packet (`M_UPDATE` word first) and keeps it to write back. The fields it
    /// alone carries are taken; one that does not decode is kept as bytes all the same.
    pub fn read_update(&mut self, update: &[u8]) {
        let raw: Arc<[u8]> = Arc::from(update);
        match &mut self.inner {
            Inner::Opaque(o) => o.raw_update = Some(raw),
            Inner::Known(k) => {
                let mut r = NetReader::new(update);
                if r.r_begin().ok() == Some(M_UPDATE) {
                    let mut object = k.object.clone();
                    if update_read(&mut object, &k.header, k.info, &mut r).is_ok() {
                        k.object = object;
                    }
                }
                k.raw_update = Some(raw);
            }
        }
    }

    /// The spawn packet: the bytes it was read from while it is unchanged, else
    /// `Spawn_Write(packet, TRUE)`.
    #[must_use]
    pub fn to_spawn_packet(&self) -> Vec<u8> {
        match &self.inner {
            Inner::Known(k) => {
                if let Some(raw) = &k.raw_spawn {
                    return raw.to_vec();
                }
                let mut w = new_writer(&k.header, false);
                let at = write_header(&k.header, &mut w);
                state_write(&k.object, &k.header, k.info.class, &mut w);
                close_state(&mut w, at);
                w.into_bytes()
            }
            Inner::Opaque(o) => {
                let Some(raw) = &o.raw_spawn else { return Vec::new() };
                if !o.header_dirty {
                    return raw.to_vec();
                }
                let mut w = new_writer(&o.header, false);
                let at = write_header(&o.header, &mut w);
                w.w(o.raw_state());
                close_state(&mut w, at);
                w.into_bytes()
            }
        }
    }

    /// The update packet `all.spawn` stores beside the spawn packet: `w_begin(M_UPDATE)` and
    /// `UPDATE_Write`, or the bytes it was read from while unchanged. `None` for an opaque
    /// entity that came without one.
    #[must_use]
    pub fn update_packet(&self) -> Option<Vec<u8>> {
        match &self.inner {
            Inner::Known(k) => {
                if let Some(raw) = &k.raw_update {
                    return Some(raw.to_vec());
                }
                let mut w = NetWriter::new(k.header.text);
                w.w_begin(M_UPDATE);
                update_write(&k.object, &k.header, k.info.class, &mut w);
                Some(w.into_bytes())
            }
            Inner::Opaque(o) => o.raw_update.as_ref().map(|r| r.to_vec()),
        }
    }

    /// The numbered values of an LTX scene section (`SaveLTX`). `None` when the entity writes
    /// raw bytes a section cannot hold (client data, a chunk) or is opaque without values.
    #[must_use]
    pub fn to_scene(&self) -> Option<Vec<IniValue>> {
        match &self.inner {
            Inner::Known(k) => {
                let mut w = new_writer(&k.header, true);
                write_header(&k.header, &mut w);
                state_write(&k.object, &k.header, k.info.class, &mut w);
                w.ini_values()
            }
            Inner::Opaque(o) => {
                let values = o.raw_ini.as_ref()?;
                if o.header_dirty {
                    return None;
                }
                Some(values.iter().map(|v| IniValue::Raw(v.to_string())).collect())
            }
        }
    }

    // ------------------------------------------------------------------ queries

    #[must_use]
    pub fn is_opaque(&self) -> bool {
        matches!(self.inner, Inner::Opaque(_))
    }

    #[must_use]
    pub fn opaque(&self) -> Option<&Opaque> {
        match &self.inner {
            Inner::Opaque(o) => Some(o),
            Inner::Known(_) => None,
        }
    }

    /// The decoded parts. `None` for an opaque entity.
    #[must_use]
    pub fn object(&self) -> Option<&Object> {
        match &self.inner {
            Inner::Known(k) => Some(&k.object),
            Inner::Opaque(_) => None,
        }
    }

    #[must_use]
    pub fn class_info(&self) -> Option<&'static ClassInfo> {
        match &self.inner {
            Inner::Known(k) => Some(k.info),
            Inner::Opaque(_) => None,
        }
    }

    /// The C++ class name, `"unknown"` for an opaque entity.
    #[must_use]
    pub fn class_name(&self) -> &'static str {
        self.class_info().map_or("unknown", |i| i.class.name())
    }

    #[must_use]
    pub fn header(&self) -> &Header {
        match &self.inner {
            Inner::Known(k) => &k.header,
            Inner::Opaque(o) => &o.header,
        }
    }

    #[must_use]
    pub fn section(&self) -> &str {
        &self.header().section
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.header().name
    }

    #[must_use]
    pub fn position(&self) -> Vec3 {
        self.header().position
    }

    #[must_use]
    pub fn angle(&self) -> Vec3 {
        self.header().angle
    }

    fn known(&self) -> Option<&Known> {
        match &self.inner {
            Inner::Known(k) => Some(k),
            Inner::Opaque(_) => None,
        }
    }

    /// The visual of a class that has one.
    #[must_use]
    pub fn visual_name(&self) -> Option<&str> {
        let k = self.known()?;
        k.info.class.has_visual().then(|| k.object.visual.name.as_str())
    }

    #[must_use]
    pub fn shapes(&self) -> &[Shape] {
        self.known().map_or(&[], |k| k.object.shapes())
    }

    /// `m_story_id`, `None` when the class has none or it is unset.
    #[must_use]
    pub fn story_id(&self) -> Option<u32> {
        let k = self.known()?;
        (k.info.class.is_alife() && k.object.alife.story_id != u32::MAX).then_some(k.object.alife.story_id)
    }

    /// `m_ini_string`.
    #[must_use]
    pub fn custom_data(&self) -> Option<&str> {
        let k = self.known()?;
        k.info.class.is_alife().then(|| k.object.alife.ini_string.as_str())
    }

    /// `m_tGraphID`.
    #[must_use]
    pub fn game_vertex_id(&self) -> Option<u16> {
        let k = self.known()?;
        k.info.class.is_alife().then_some(k.object.alife.graph_id)
    }

    /// `m_tNodeID`.
    #[must_use]
    pub fn level_vertex_id(&self) -> Option<u32> {
        let k = self.known()?;
        if k.info.class == Class::Temporary {
            return k.object.temporary().map(|t| t.node_id);
        }
        k.info.class.is_alife().then_some(k.object.alife.node_id)
    }

    #[must_use]
    pub fn level_changer(&self) -> Option<LevelChangerDestination> {
        let lc = self.known()?.object.level_changer()?;
        Some(LevelChangerDestination {
            level: lc.level_to_change.clone(),
            point: lc.level_point_to_change.clone(),
            graph_id: lc.next_graph_id,
            node_id: lc.next_node_id,
            position: lc.next_position,
            direction: lc.angles,
            silent: lc.silent_mode,
        })
    }

    // ------------------------------------------------------------------ edits

    fn header_mut(&mut self) -> Result<&mut Header> {
        match &mut self.inner {
            Inner::Known(k) => {
                k.raw_spawn = None;
                k.raw_update = None;
                Ok(&mut k.header)
            }
            Inner::Opaque(o) => {
                if !o.header.complete() || o.raw_spawn.is_none() {
                    return Err(EntityError::Opaque(o.reason.text()));
                }
                o.header_dirty = true;
                Ok(&mut o.header)
            }
        }
    }

    fn known_mut(&mut self) -> Result<&mut Known> {
        match &mut self.inner {
            Inner::Known(k) => {
                k.raw_spawn = None;
                k.raw_update = None;
                Ok(k)
            }
            Inner::Opaque(o) => Err(EntityError::Opaque(o.reason.text())),
        }
    }

    /// `set_name_replace`.
    pub fn set_name(&mut self, name: &str) -> Result<()> {
        self.header_mut()?.name = SmolStr::new(name);
        Ok(())
    }

    pub fn set_position(&mut self, position: Vec3) -> Result<()> {
        self.header_mut()?.position = position;
        Ok(())
    }

    pub fn set_angle(&mut self, angle: Vec3) -> Result<()> {
        self.header_mut()?.angle = angle;
        Ok(())
    }

    pub fn set_ids(&mut self, id: u16, id_parent: u16) -> Result<()> {
        let h = self.header_mut()?;
        h.id = id;
        h.id_parent = id_parent;
        Ok(())
    }

    /// `assign_shapes`: for a class with a shape.
    pub fn set_shapes(&mut self, shapes: &[Shape]) -> Result<()> {
        let k = self.known_mut()?;
        if !k.info.class.has_shape() {
            return Err(EntityError::NoProperty("shapes".to_owned()));
        }
        k.object.shapes = Shapes::from_slice(shapes);
        Ok(())
    }

    /// A climable object's game material (`CSE_ALifeObjectClimable::material`), what the level
    /// build gives the ladders it makes (`set_additional_info`).
    pub fn set_climable_material(&mut self, material: &str) -> Result<()> {
        let k = self.known_mut()?;
        if k.info.class != crate::class::Class::Climable {
            return Err(EntityError::NoProperty("material".to_owned()));
        }
        k.object.climable_mut().material = SmolStr::new(material);
        Ok(())
    }

    /// The graph and level vertex a level build assigns.
    pub fn set_vertices(&mut self, game_vertex: u16, level_vertex: u32, distance: f32) -> Result<()> {
        let k = self.known_mut()?;
        if !k.info.class.is_alife() {
            return Err(EntityError::NoProperty("vertices".to_owned()));
        }
        k.object.alife.graph_id = game_vertex;
        k.object.alife.node_id = level_vertex;
        k.object.alife.distance = distance;
        Ok(())
    }

    /// Where an anomalous zone's artefact points start in the spawn's point list
    /// (`m_artefact_position_offset`, set by the spawn constructor).
    pub fn set_artefact_offset(&mut self, offset: u32) -> Result<()> {
        if self.known().and_then(|k| k.object.zone()).is_none() {
            return Err(EntityError::NoProperty("artefact position offset".to_owned()));
        }
        self.known_mut()?.object.zone_mut().artefact_position_offset = offset;
        Ok(())
    }

    /// Where a level changer leads: the game vertex, its level vertex, the position and the
    /// direction of the graph point it names (`fill_level_changers`).
    pub fn set_level_changer_destination(
        &mut self,
        graph_id: u16,
        node_id: u32,
        position: Vec3,
        angles: Vec3,
    ) -> Result<()> {
        if self.known().and_then(|k| k.object.level_changer()).is_none() {
            return Err(EntityError::NoProperty("level changer destination".to_owned()));
        }
        let lc = self.known_mut()?.object.level_changer_mut();
        lc.next_graph_id = graph_id;
        lc.next_node_id = node_id;
        lc.next_position = position;
        lc.angles = angles;
        Ok(())
    }

    /// The properties `FillProps` lists, in its order. An opaque entity lists its header,
    /// read-only.
    #[must_use]
    pub fn properties(&self, ctx: &PropContext) -> Vec<Prop> {
        match &self.inner {
            Inner::Known(k) => props::list(&k.header, k.info, &k.object, ctx),
            Inner::Opaque(o) => props::list_opaque(&o.header, o.reason),
        }
    }

    /// Sets the property at `path` (as [`Prop::path`] names it). Numbers are clamped to the
    /// property's limits, as the property sheet clamps them.
    pub fn set(&mut self, path: &str, value: &PropValue, ctx: &PropContext) -> Result<SetEffect> {
        let k = match &mut self.inner {
            Inner::Known(k) => k,
            Inner::Opaque(o) => {
                return Err(if props::list_opaque(&o.header, o.reason).iter().any(|p| p.path == path) {
                    EntityError::ReadOnly(path.to_owned())
                } else {
                    EntityError::NoProperty(path.to_owned())
                });
            }
        };
        let list = props::list(&k.header, k.info, &k.object, ctx);
        let prop = list
            .into_iter()
            .find(|p| p.path == path)
            .ok_or_else(|| EntityError::NoProperty(path.to_owned()))?;
        if prop.read_only {
            return Err(EntityError::ReadOnly(path.to_owned()));
        }
        let effect = props::apply(&mut k.header, &mut k.object, &prop, value, ctx)?;
        k.raw_spawn = None;
        k.raw_update = None;
        Ok(effect)
    }

    /// What `se_smart_cover:FillProps` does to the state when the property sheet is shown: a
    /// new description starts the loopholes over and every loophole of it is switched on.
    pub fn sync_script(&mut self, ctx: &PropContext) {
        let Inner::Known(k) = &mut self.inner else { return };
        let Some(cover) = k.object.cover() else { return };
        let description = cover.description.clone();
        let ids: Vec<&str> = ctx
            .smart_covers
            .get(&description)
            .map(|d| d.loopholes.iter().map(SmolStr::as_str).collect())
            .unwrap_or_default();
        let before = k.object.script.clone();
        k.object.script.sync_loopholes(&description, &ids);
        if k.object.script != before {
            k.raw_spawn = None;
            k.raw_update = None;
        }
    }
}

fn collect_ini(values: &dyn IniSource) -> Arc<[SmolStr]> {
    let mut out = Vec::new();
    let mut i = 1u32;
    while let Some(v) = values.value(i) {
        out.push(SmolStr::new(v));
        i += 1;
    }
    Arc::from(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create::fixtures::settings;
    use crate::model::Shape;
    use crate::props::PropContext;

    fn round_trip(section: &str) {
        let s = settings();
        let mut e = Entity::create(section, &s).unwrap();
        e.set_name(&format!("{section}_0000")).unwrap();
        e.set_position(Vec3::new(1.0, 2.0, 3.0)).unwrap();
        if e.class_info().unwrap().class.has_shape() {
            e.set_shapes(&[Shape::Sphere { center: Vec3::ZERO, radius: 2.0 }]).unwrap();
        }
        let first = e.to_spawn_packet();
        let back = Entity::from_spawn_packet(&first, &s).unwrap();
        assert!(!back.is_opaque(), "{section}: {:?}", back.opaque().map(|o| (o.reason, &o.detail)));
        assert_eq!(back.section(), section);
        // an untouched entity writes its own bytes; a touched one writes them anew
        assert_eq!(back.to_spawn_packet(), first, "{section}");
        let mut edited = back.clone();
        edited.set_position(Vec3::new(1.0, 2.0, 3.0)).unwrap();
        assert_eq!(edited.to_spawn_packet(), first, "{section}: re-encoded");
        let update = e.update_packet().unwrap();
        let both = Entity::from_packets(&first, &update, &s).unwrap();
        assert_eq!(both.update_packet().unwrap(), update);
        let mut touched = both.clone();
        touched.set_name(&format!("{section}_0000")).unwrap();
        assert_eq!(touched.update_packet().unwrap(), update, "{section}: update");
    }

    #[test]
    fn every_class_family_writes_reads_and_writes_the_same_bytes() {
        for section in [
            "spectator",
            "graph_point",
            "rocket",
            "script_object",
            "skeleton_object",
            "space_restrictor",
            "level_changer",
            "old_level_changer",
            "smart_terrain",
            "smart_zone",
            "physic_object",
            "lamp",
            "projector",
            "helicopter",
            "car",
            "breakable",
            "climable",
            "mounted",
            "team_base",
            "inventory_box",
            "zone",
            "anomaly",
            "torrid",
            "zone_bfuzz.x",
            "crow",
            "phantom",
            "squad",
            "plain_squad",
            "smart_cover",
            "animpoint",
            "medkit",
            "bread",
            "torch",
            "ammo_9x18",
            "knife",
            "pistol",
            "rifle",
            "shotgun",
            "auto_shotgun",
            "detector",
            "artefact",
            "plain_artefact",
            "pda",
            "document",
            "grenade",
            "explosive",
            "bolt",
            "outfit",
            "helmet",
            "trader",
            "actor",
            "dog",
            "plain_dog",
            "phantom_dog",
            "rat",
            "stalker",
        ] {
            round_trip(section);
        }
    }

    #[test]
    fn the_header_is_spawn_write() {
        let s = settings();
        let e = Entity::create("medkit", &s).unwrap();
        let p = e.to_spawn_packet();
        let mut r = NetReader::new(&p);
        assert_eq!(r.r_u16().unwrap(), M_SPAWN);
        assert_eq!(r.r_stringz().unwrap(), "medkit");
        assert_eq!(r.r_stringz().unwrap(), "");
        assert_eq!((r.r_u8().unwrap(), r.r_u8().unwrap()), (0, 0xFE));
        r.r_advance(24 + 8).unwrap();
        assert_eq!(r.r_u16().unwrap(), M_SPAWN_VERSION | M_SPAWN_OBJECT_LOCAL);
        assert_eq!(r.r_u16().unwrap(), SPAWN_VERSION);
        assert_eq!(r.r_u16().unwrap(), 0xFFFF);
        assert_eq!(r.r_u16().unwrap(), 12);
        assert_eq!(r.r_u16().unwrap(), 0);
        assert_eq!(r.r_u16().unwrap(), 0xFFFF);
        let size = usize::from(r.r_u16().unwrap());
        assert_eq!(size, r.r_elapsed() + 2);
    }

    #[test]
    fn an_unknown_class_is_kept_byte_for_byte() {
        let s = settings();
        let mut p = Entity::create("medkit", &s).unwrap().to_spawn_packet();
        // the section name becomes one the table has no class for
        let at = p.iter().position(|&b| b == b'm').unwrap();
        p.splice(at..at + 6, b"nothin".iter().copied());
        let e = Entity::from_spawn_packet(&p, &s).unwrap();
        let o = e.opaque().unwrap();
        assert_eq!(o.reason, OpaqueReason::UnknownClass);
        assert_eq!(e.to_spawn_packet(), p);
        assert_eq!(e.class_name(), "unknown");
        assert!(e.properties(&PropContext::default()).iter().all(|p| p.read_only));
        let mut moved = e.clone();
        moved.set_position(Vec3::new(9.0, 0.0, 0.0)).unwrap();
        let again = moved.to_spawn_packet();
        assert_eq!(again.len(), p.len());
        assert_eq!(Entity::from_spawn_packet(&again, &s).unwrap().position(), Vec3::new(9.0, 0.0, 0.0));
        assert!(
            moved
                .set(
                    &format!("{}\\Custom data", "nothin"),
                    &PropValue::Text("x".into()),
                    &PropContext::default()
                )
                .is_err()
        );
    }

    #[test]
    fn a_newer_version_and_a_broken_state_are_kept() {
        let s = settings();
        let p = Entity::create("stalker", &s).unwrap().to_spawn_packet();
        let mut newer = p.clone();
        let at = newer.windows(2).position(|w| w == SPAWN_VERSION.to_le_bytes()).unwrap();
        newer[at..at + 2].copy_from_slice(&200u16.to_le_bytes());
        let e = Entity::from_spawn_packet(&newer, &s).unwrap();
        assert_eq!(e.opaque().unwrap().reason, OpaqueReason::NewerVersion);
        assert_eq!(e.to_spawn_packet(), newer);

        let cut = &p[..p.len() - 3];
        let e = Entity::from_spawn_packet(cut, &s).unwrap();
        assert!(e.is_opaque());
        assert_eq!(e.to_spawn_packet(), cut);

        let mut extra = p.clone();
        extra.push(7);
        let e = Entity::from_spawn_packet(&extra, &s).unwrap();
        assert_eq!(e.opaque().unwrap().reason, OpaqueReason::Lossy);
        assert_eq!(e.to_spawn_packet(), extra);

        assert!(Entity::from_spawn_packet(&p[..10], &s).is_err());
        assert!(matches!(Entity::from_spawn_packet(&[0, 0, 0], &s), Err(EntityError::NotSpawn(0))));
    }

    #[test]
    fn accessors_read_the_parts() {
        let s = settings();
        let mut e = Entity::create("level_changer", &s).unwrap();
        assert_eq!(e.class_name(), "CSE_ALifeLevelChanger");
        assert_eq!(e.game_vertex_id(), Some(u16::MAX));
        assert_eq!(e.story_id(), None);
        let ctx = PropContext::default();
        e.set("level_changer\\Level to change", &PropValue::Text("l02_garbage".into()), &ctx).unwrap();
        e.set("level_changer\\Silent mode", &PropValue::Bool(true), &ctx).unwrap();
        let d = e.level_changer().unwrap();
        assert_eq!((d.level.as_str(), d.silent), ("l02_garbage", true));
        let back = Entity::from_spawn_packet(&e.to_spawn_packet(), &s).unwrap();
        assert_eq!(back.level_changer(), Some(d));
        assert_eq!(Entity::create("physic_object", &s).unwrap().visual_name(), Some("dynamics\\barrel"));
        assert_eq!(Entity::create("space_restrictor", &s).unwrap().visual_name(), None);
    }

    #[test]
    fn a_scene_section_reads_back() {
        let s = settings();
        let e = Entity::create("pistol", &s).unwrap();
        let values = e.to_scene().unwrap();
        let text: Vec<String> = values.iter().map(IniValue::to_ltx).collect();
        let back = Entity::from_scene(&text, &s).unwrap();
        assert!(!back.is_opaque());
        assert_eq!(back.to_spawn_packet(), e.to_spawn_packet());
        // client data is raw bytes: a section cannot hold it
        let mut with_data = e.clone();
        with_data.header_mut().unwrap().client_data = vec![1, 2];
        assert!(with_data.to_scene().is_none());
    }

    #[test]
    fn an_old_version_is_read_and_written_as_today() {
        let s = settings();
        // version 118 of a space restrictor: no game type word, the same state
        let p = Entity::create("space_restrictor", &s).unwrap().to_spawn_packet();
        let mut r = NetReader::new(&p);
        r.r_u16().unwrap();
        r.r_stringz().unwrap();
        r.r_stringz().unwrap();
        r.r_advance(2 + 24 + 8 + 2).unwrap();
        let version_at = r.r_tell();
        let mut old = p[..version_at].to_vec();
        old.extend_from_slice(&118u16.to_le_bytes());
        old.extend_from_slice(&p[version_at + 4..]);
        let e = Entity::from_spawn_packet(&old, &s).unwrap();
        assert!(!e.is_opaque());
        assert_eq!(e.header().game_type, 0xFFFF);
        let mut moved = e.clone();
        moved.set_name("").unwrap();
        assert_eq!(moved.to_spawn_packet(), p);
    }
}
