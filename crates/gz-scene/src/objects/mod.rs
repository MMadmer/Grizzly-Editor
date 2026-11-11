//! The sixteen classes' data and their part of the scene files.
//!
//! [`read_ltx`] / [`write_ltx`] are `EScene::ReadObjectLTX` / `SaveObjectLTX` (a `clsid` key and
//! the object's keys in one section, the objects a group or a spawn point holds in sections
//! under it); [`read_stream`] / [`write_stream`] are `ReadObjectStream` / `SaveObjectStream`
//! (`CHUNK_OBJECT_CLASS` and `CHUNK_OBJECT_BODY`). A body of a version the C++ refuses is kept
//! ([`KeptObject`]) and written back as it was read.

mod shape;
mod simple;
mod spawn;
mod way;

use std::sync::Arc;

use gz_base::{Blob, SmolStr};
use gz_chunk::{Reader, Writer};
use gz_entities::Settings;

pub use shape::{
    CF_BOX, CF_SPHERE, FV_EMITTER, FV_OCCLUSION, FogData, SHAPE_COMMON, SHAPE_LEVEL_BOUND, ShapeData,
    ShapeDef, SoundEnvData,
};
pub use simple::{
    Body, Fuzzy, GlowData, KeptObject, LT_DIRECT, LT_POINT, LT_SPOT, LightData, ParticlesData,
    SO_USE_SURFACE, SO_VISUAL_MODE, SceneObjectData, SoundSrcData, SurfaceOverride, light_flags,
};
pub use spawn::{
    ENVMOD_CHOOSE_NAME, EnvMod, ORDER_HOSTILE, ORDER_RUN, PLAYERSTART_CHOOSE_NAME, PointKind,
    RPOINT_CHOOSE_NAME, RPoint, SpawnData, SpawnEntity, XmsOrder, env_flags,
};
pub use way::{
    PortalData, SectorData, WAYPOINT_RADIUS, WAYPOINT_SIZE, WayData, WayPoint, fcolor_from, fcolor_get,
};

use crate::SceneError;
use crate::class::ObjClass;
use crate::ltx::{IniReader, IniWriter};
use crate::math::{Aabb, Transform, Vec3, point_box, xform_box};
use crate::model::{MotionRaw, ObjData, ObjId, Object, SceneState, co};

pub const CHUNK_OBJECT_CLASS: u32 = 0x7703;
pub const CHUNK_OBJECT_BODY: u32 = 0x7777;
const CO_CHUNK_TRANSFORM: u32 = 0xF903;
const CO_CHUNK_MOTION: u32 = 0xF905;
const CO_CHUNK_FLAGS: u32 = 0xF906;
const CO_CHUNK_NAME: u32 = 0xF907;
const CO_CHUNK_MOTION_PARAM: u32 = 0xF908;
pub const GROUP_VERSION: u32 = 0x12;

/// `CGroupObject`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct GroupData {
    /// `m_ReferenceName_`: the `.group` file it was made from.
    pub reference: SmolStr,
    /// The objects it holds, in order. They stay in the scene's lists.
    pub members: Vec<ObjId>,
}

impl GroupData {
    /// `SetRefName`: light presets go to `group.part1`.
    #[must_use]
    pub fn save_id(&self) -> usize {
        usize::from(self.reference.contains("light_preset"))
    }
}

/// What reading an object needs and what it noticed.
pub struct ReadCx<'a> {
    /// The linked game's configs: spawn entities decode with them.
    pub settings: Option<&'a dyn Settings>,
    pub warnings: Vec<String>,
}

impl<'a> ReadCx<'a> {
    #[must_use]
    pub fn new(settings: Option<&'a dyn Settings>) -> Self {
        Self { settings, warnings: Vec::new() }
    }
}

/// An object as read, with the objects a group holds.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub obj: Object,
    pub members: Vec<Loaded>,
}

/// A new object of a class as its tool's `CreateObject` makes it (no spawn entity: that takes
/// the game's configs).
#[must_use]
pub fn default_data(class: ObjClass) -> ObjData {
    match class {
        ObjClass::SceneObject => ObjData::SceneObject(SceneObjectData::default()),
        ObjClass::Light => ObjData::Light(LightData::default()),
        ObjClass::Glow => ObjData::Glow(GlowData::default()),
        ObjClass::Shape => ObjData::Shape(ShapeData::default()),
        ObjClass::SoundSrc => ObjData::SoundSrc(SoundSrcData::default()),
        ObjClass::SoundEnv => ObjData::SoundEnv(SoundEnvData::default()),
        ObjClass::SpawnPoint => ObjData::Spawn(Box::new(SpawnData::of_kind(PointKind::PlayerStart))),
        ObjClass::Way => ObjData::Way(WayData::default()),
        ObjClass::Sector => ObjData::Sector(SectorData::default()),
        ObjClass::Portal => ObjData::Portal(PortalData::default()),
        ObjClass::Group => ObjData::Group(GroupData::default()),
        ObjClass::Particles => ObjData::Particles(ParticlesData::default()),
        ObjClass::FogVolume => ObjData::FogVolume(FogData::default()),
        ObjClass::DetailObjects | ObjClass::AiMap | ObjClass::Wallmark => {
            ObjData::Kept(KeptObject::Stream { why: "not an object class".into(), body: Blob::from(&[][..]) })
        }
    }
}

fn kept_sections(r: &IniReader, sect: &str) -> Vec<(String, Vec<(String, String)>)> {
    let lower = sect.to_lowercase();
    let prefix = format!("{lower}_");
    r.sections()
        .filter(|s| *s == lower || s.starts_with(&prefix))
        .map(|s| (s[lower.len()..].to_owned(), r.lines(s)))
        .collect()
}

/// `ReadObjectLTX`.
pub fn read_ltx(r: &IniReader, sect: &str, cx: &mut ReadCx<'_>) -> Result<Loaded, SceneError> {
    if !r.section_exist(sect) || !r.line_exist(sect, "clsid") {
        return Err(SceneError::load(format!("{}: object section '{sect}' is invalid", r.file())));
    }
    let class =
        ObjClass::from_id(r.r_u32(sect, "clsid")?).filter(|c| c.is_object_tool()).ok_or_else(|| {
            SceneError::load(format!("{}: object section '{sect}' names no object class", r.file()))
        })?;
    let mut obj = Object::new(class, "", default_data(class));
    obj.set_co_flags(r.r_u32(sect, "co_flags")?);
    obj.name = r.r_string(sect, "name")?.into();
    obj.xform = Transform {
        position: r.r_fvector3(sect, "position")?,
        rotation: r.r_fvector3(sect, "rotation")?,
        scale: r.r_fvector3(sect, "scale")?,
    };
    // the LTX format has no object motion: the flag goes
    obj.flags.co &= !co::MOTION;
    let mut members = Vec::new();
    let refused = |why: String, cx: &mut ReadCx<'_>| {
        cx.warnings.push(format!("'{sect}': {why} The object is kept as it was read."));
        ObjData::Kept(KeptObject::Ltx { why, sections: kept_sections(r, sect) })
    };
    obj.data = match class {
        ObjClass::SceneObject => ObjData::SceneObject(SceneObjectData::read_ltx(r, sect)?),
        ObjClass::Light => match LightData::read_ltx(r, sect)? {
            Body::Read(d) => ObjData::Light(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::Glow => match GlowData::read_ltx(r, sect)? {
            Body::Read(d) => ObjData::Glow(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::Shape => ObjData::Shape(ShapeData::read_ltx(r, sect)?),
        ObjClass::SoundSrc => match SoundSrcData::read_ltx(r, sect)? {
            Body::Read(d) => ObjData::SoundSrc(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::SoundEnv => match SoundEnvData::read_ltx(r, sect)? {
            Body::Read(d) => ObjData::SoundEnv(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::Way => match WayData::read_ltx(r, sect)? {
            Body::Read(d) => ObjData::Way(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::Sector => match SectorData::read_ltx(r, sect)? {
            Body::Read(d) => ObjData::Sector(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::Portal => match PortalData::read_ltx(r, sect)? {
            Body::Read(d) => ObjData::Portal(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::Particles => ObjData::Particles(ParticlesData::read_ltx(r, sect)?),
        ObjClass::FogVolume => ObjData::FogVolume(FogData::read_ltx(r, sect)?),
        ObjClass::SpawnPoint => match SpawnData::read_ltx(r, sect, cx.settings)? {
            Body::Refused(w) => refused(w, cx),
            Body::Read(mut d) => {
                if r.line_exist(sect, "attached_count") {
                    let n = r.r_u32(sect, "attached_count")?;
                    for i in 0..n.min(1) {
                        let a = read_ltx(r, &format!("{sect}_attached_{i}"), cx)?;
                        if a.obj.class != ObjClass::Shape {
                            return Err(SceneError::load(format!(
                                "'{sect}': only a shape can be attached to a spawn point"
                            )));
                        }
                        d.attached = Some(Arc::new(a.obj));
                    }
                }
                if let Some(a) = &d.attached {
                    obj.xform.scale = a.xform.scale;
                }
                ObjData::Spawn(Box::new(d))
            }
        },
        ObjClass::Group => {
            let version = r.r_u32(sect, "version")?;
            let reference = r.opt_string(sect, "ref_name").unwrap_or_default();
            if version < 0x11 || !r.line_exist(sect, "ingroup_count") || reference.is_empty() {
                refused("CGroupObject: unsupported file version, or no objects or reference.".into(), cx)
            } else {
                let n = r.r_u32(sect, "ingroup_count")?;
                for i in 0..n.min(1 << 16) {
                    let mut m = read_ltx(r, &format!("{sect}_ingroup_{i}"), cx)?;
                    m.obj.flags.co |= co::IN_GROUP;
                    if version < 0x12 {
                        m.obj.flags.co |= co::IN_GROUP_UNIQUE;
                    }
                    members.push(m);
                }
                if members.is_empty() {
                    refused("CGroupObject: the group holds no objects.".into(), cx)
                } else {
                    ObjData::Group(GroupData { reference: reference.into(), members: Vec::new() })
                }
            }
        }
        ObjClass::DetailObjects | ObjClass::AiMap | ObjClass::Wallmark => unreachable_class(),
    };
    Ok(Loaded { obj, members })
}

fn unreachable_class() -> ObjData {
    ObjData::Kept(KeptObject::Stream { why: "not an object class".into(), body: Blob::from(&[][..]) })
}

/// `SaveObjectLTX`: `clear` takes bits out of the `co_flags` written (a group's members are
/// saved without `flObjectInGroup`).
pub fn write_ltx(
    w: &mut IniWriter,
    sect: &str,
    obj: &Object,
    state: &SceneState,
    clear: u32,
) -> Result<(), SceneError> {
    w.w_u32(sect, "clsid", obj.class.id());
    if let ObjData::Kept(KeptObject::Ltx { sections, .. }) = &obj.data {
        for (suffix, lines) in sections {
            let s = format!("{sect}{suffix}");
            for (k, v) in lines {
                w.w_raw(&s, k, v);
            }
        }
        return Ok(());
    }
    w.w_u32(sect, "co_flags", obj.co_flags() & !clear);
    w.w_string(sect, "name", &obj.name);
    w.w_fvector3(sect, "position", obj.xform.position);
    w.w_fvector3(sect, "rotation", obj.xform.rotation);
    w.w_fvector3(sect, "scale", obj.xform.scale);
    match &obj.data {
        ObjData::SceneObject(d) => d.write_ltx(w, sect),
        ObjData::Light(d) => d.write_ltx(w, sect),
        ObjData::Glow(d) => d.write_ltx(w, sect),
        ObjData::Shape(d) => d.write_ltx(w, sect),
        ObjData::SoundSrc(d) => d.write_ltx(w, sect),
        ObjData::SoundEnv(d) => d.write_ltx(w, sect),
        ObjData::Way(d) => d.write_ltx(w, sect),
        ObjData::Sector(d) => d.write_ltx(w, sect),
        ObjData::Portal(d) => d.write_ltx(w, sect),
        ObjData::Particles(d) => d.write_ltx(w, sect),
        ObjData::FogVolume(d) => d.write_ltx(w, sect),
        ObjData::Spawn(d) => {
            if let Some(a) = &d.attached {
                write_ltx(w, &format!("{sect}_attached_0"), a, state, 0)?;
                w.w_u32(sect, "attached_count", 1);
            }
            d.write_ltx(w, sect, &obj.name)?;
        }
        ObjData::Group(g) => {
            w.w_u32(sect, "version", GROUP_VERSION);
            let mut n = 0u32;
            for id in &g.members {
                if let Some(m) = state.get(*id) {
                    write_ltx(w, &format!("{sect}_ingroup_{n}"), m, state, co::IN_GROUP)?;
                    n += 1;
                }
            }
            w.w_u32(sect, "ingroup_count", n);
            w.w_string(sect, "ref_name", &g.reference);
        }
        ObjData::Kept(KeptObject::Stream { .. }) => {
            return Err(SceneError::Save(format!(
                "'{}' came from a binary scene in a form that cannot be written as text.",
                obj.name
            )));
        }
        ObjData::Kept(KeptObject::Ltx { .. }) => {}
    }
    Ok(())
}

/// `ReadObjectsStream`: the numbered chunks of a list.
pub fn read_stream_list(list: &Reader<'_>, cx: &mut ReadCx<'_>) -> Result<Vec<Loaded>, SceneError> {
    let mut out = Vec::new();
    for i in 0u32.. {
        let Some(c) = list.open_chunk(i)? else { break };
        out.push(read_stream(&c.reader(), cx)?);
    }
    Ok(out)
}

/// `ReadObjectStream`.
pub fn read_stream(r: &Reader<'_>, cx: &mut ReadCx<'_>) -> Result<Loaded, SceneError> {
    let class_id = r
        .open_chunk(CHUNK_OBJECT_CLASS)?
        .ok_or_else(|| SceneError::load("an object has no class chunk"))?
        .reader()
        .r_u32()?;
    let class = ObjClass::from_id(class_id)
        .filter(|c| c.is_object_tool())
        .ok_or_else(|| SceneError::load(format!("an object names class {class_id}, which has no objects")))?;
    let body_chunk =
        r.open_chunk(CHUNK_OBJECT_BODY)?.ok_or_else(|| SceneError::load("an object has no body"))?;
    let raw = Blob::from(body_chunk.bytes());
    let b = body_chunk.reader();
    let mut obj = Object::new(class, "", default_data(class));
    let flags = b
        .open_chunk(CO_CHUNK_FLAGS)?
        .ok_or_else(|| SceneError::load("an object has no flags"))?
        .reader()
        .r_u32()?;
    obj.set_co_flags(flags);
    obj.name = b
        .open_chunk(CO_CHUNK_NAME)?
        .ok_or_else(|| SceneError::load("an object has no name"))?
        .reader()
        .r_stringz()?
        .into();
    let mut has_transform = false;
    if let Some(c) = b.open_chunk(CO_CHUNK_TRANSFORM)? {
        let mut q = c.reader();
        obj.xform = Transform { position: q.r_vec3()?, rotation: q.r_vec3()?, scale: q.r_vec3()? };
        has_transform = true;
    }
    if let Some(m) = b.open_chunk(CO_CHUNK_MOTION)? {
        let t = match b.open_chunk(CO_CHUNK_MOTION_PARAM)? {
            Some(p) => Some(p.reader().r_f32()?),
            None => None,
        };
        obj.motion = Some(Arc::new(MotionRaw { motion: Blob::from(m.bytes()), t_current: t }));
    }
    let mut members = Vec::new();
    let refused = |why: String, cx: &mut ReadCx<'_>| {
        cx.warnings.push(format!("'{}': {why} The object is kept as it was read.", obj.name));
        ObjData::Kept(KeptObject::Stream { why, body: raw.clone() })
    };
    let mut position: Option<Vec3> = None;
    let data = match class {
        ObjClass::SceneObject => {
            let mut placement = None;
            match SceneObjectData::read_stream(&b, &mut placement)? {
                Body::Read(d) => {
                    if let (Some([p, rot, s]), false) = (placement, has_transform) {
                        obj.xform = Transform { position: p, rotation: rot, scale: s };
                    }
                    ObjData::SceneObject(d)
                }
                Body::Refused(w) => refused(w, cx),
            }
        }
        ObjClass::Light => match LightData::read_stream(&b, &mut position)? {
            Body::Read(d) => ObjData::Light(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::Glow => match GlowData::read_stream(&b, &mut position)? {
            Body::Read(d) => ObjData::Glow(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::Shape => match ShapeData::read_stream(&b)? {
            Body::Read(d) => ObjData::Shape(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::SoundSrc => match SoundSrcData::read_stream(&b)? {
            Body::Read(d) => ObjData::SoundSrc(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::SoundEnv => match SoundEnvData::read_stream(&b)? {
            Body::Read(d) => ObjData::SoundEnv(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::Way => match WayData::read_stream(&b)? {
            Body::Read(d) => ObjData::Way(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::Sector => match SectorData::read_stream(&b)? {
            Body::Read(d) => ObjData::Sector(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::Portal => match PortalData::read_stream(&b)? {
            Body::Read(d) => ObjData::Portal(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::Particles => match ParticlesData::read_stream(&b)? {
            Body::Read(d) => ObjData::Particles(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::FogVolume => match FogData::read_stream(&b)? {
            Body::Read(d) => ObjData::FogVolume(d),
            Body::Refused(w) => refused(w, cx),
        },
        ObjClass::SpawnPoint => match SpawnData::read_stream(&b, cx.settings)? {
            Body::Refused(w) => refused(w, cx),
            Body::Read(mut d) => {
                if let Some(list) = b.open_chunk(0xE421)? {
                    for a in read_stream_list(&list.reader(), cx)? {
                        if a.obj.class != ObjClass::Shape {
                            return Err(SceneError::load("only a shape can be attached to a spawn point"));
                        }
                        d.attached = Some(Arc::new(a.obj));
                    }
                }
                if let Some(a) = &d.attached {
                    obj.xform.scale = a.xform.scale;
                }
                ObjData::Spawn(Box::new(d))
            }
        },
        ObjClass::Group => {
            let version = match b.find_chunk(0) {
                Some((v, false)) if v.len() == 2 => u16::from_le_bytes([v[0], v[1]]),
                _ => 0,
            };
            let reference = match b.open_chunk(4)? {
                Some(c) if c.bytes().last() == Some(&0) && c.bytes().first().is_some_and(|b| *b != 0) => {
                    c.reader().r_stringz()?
                }
                _ => String::new(),
            };
            if version < 0x11 || reference.is_empty() {
                refused("CGroupObject: unsupported file version, or no reference.".into(), cx)
            } else {
                if let Some(list) = b.open_chunk(1)? {
                    for mut m in read_stream_list(&list.reader(), cx)? {
                        m.obj.flags.co |= co::IN_GROUP;
                        if version < 0x12 {
                            m.obj.flags.co |= co::IN_GROUP_UNIQUE;
                        }
                        members.push(m);
                    }
                }
                if members.is_empty() {
                    refused("CGroupObject: the group holds no objects.".into(), cx)
                } else {
                    ObjData::Group(GroupData { reference: reference.into(), members: Vec::new() })
                }
            }
        }
        ObjClass::DetailObjects | ObjClass::AiMap | ObjClass::Wallmark => unreachable_class(),
    };
    obj.data = data;
    if let Some(p) = position {
        obj.xform.position = p;
    }
    Ok(Loaded { obj, members })
}

/// `SaveObjectsStream`: numbered chunks.
pub fn write_stream_list<'a>(
    w: &mut Writer,
    objects: impl IntoIterator<Item = &'a Object>,
    state: &SceneState,
    clear: u32,
) -> Result<(), SceneError> {
    for (i, o) in objects.into_iter().enumerate() {
        w.open_chunk(u32::try_from(i).unwrap_or(u32::MAX));
        write_stream(w, o, state, clear)?;
        w.close_chunk();
    }
    Ok(())
}

/// `SaveObjectStream`.
pub fn write_stream(w: &mut Writer, obj: &Object, state: &SceneState, clear: u32) -> Result<(), SceneError> {
    w.chunk(CHUNK_OBJECT_CLASS, |w| w.w_u32(obj.class.id()));
    if let ObjData::Kept(KeptObject::Stream { body, .. }) = &obj.data {
        w.w_chunk(CHUNK_OBJECT_BODY, body);
        return Ok(());
    }
    let mut body = Writer::new();
    body.chunk(CO_CHUNK_FLAGS, |w| w.w_u32(obj.co_flags() & !clear));
    body.chunk(CO_CHUNK_NAME, |w| w.w_stringz(&obj.name));
    body.chunk(CO_CHUNK_TRANSFORM, |w| {
        w.w_vec3(obj.xform.position);
        w.w_vec3(obj.xform.rotation);
        w.w_vec3(obj.xform.scale);
    });
    if obj.flags.co & co::MOTION != 0
        && let Some(m) = &obj.motion
    {
        body.w_chunk(CO_CHUNK_MOTION, &m.motion);
        body.chunk(CO_CHUNK_MOTION_PARAM, |w| w.w_f32(m.t_current.unwrap_or(0.0)));
    }
    match &obj.data {
        ObjData::SceneObject(d) => d.write_stream(&mut body),
        ObjData::Light(d) => d.write_stream(&mut body),
        ObjData::Glow(d) => d.write_stream(&mut body),
        ObjData::Shape(d) => d.write_stream(&mut body),
        ObjData::SoundSrc(d) => d.write_stream(&mut body),
        ObjData::SoundEnv(d) => d.write_stream(&mut body),
        ObjData::Way(d) => d.write_stream(&mut body),
        ObjData::Sector(d) => d.write_stream(&mut body),
        ObjData::Portal(d) => d.write_stream(&mut body),
        ObjData::Particles(d) => d.write_stream(&mut body),
        ObjData::FogVolume(d) => d.write_stream(&mut body),
        ObjData::Spawn(d) => {
            body.chunk(0xE411, |w| w.w_u16(0x17));
            if let Some(a) = &d.attached {
                body.open_chunk(0xE421);
                write_stream_list(&mut body, [a.as_ref()], state, 0)?;
                body.close_chunk();
            }
            d.write_stream(&mut body, &obj.name)?;
        }
        ObjData::Group(g) => {
            body.chunk(0, |w| w.w_u16(GROUP_VERSION as u16));
            let members: Vec<&Object> =
                g.members.iter().filter_map(|id| state.get(*id).map(AsRef::as_ref)).collect();
            body.open_chunk(1);
            write_stream_list(&mut body, members, state, co::IN_GROUP)?;
            body.close_chunk();
            body.chunk(4, |w| w.w_stringz(&g.reference));
        }
        ObjData::Kept(KeptObject::Ltx { .. }) => {
            return Err(SceneError::Save(format!(
                "'{}' came from a text scene in a form that cannot be written as a stream.",
                obj.name
            )));
        }
        ObjData::Kept(KeptObject::Stream { .. }) => {}
    }
    w.w_chunk(CHUNK_OBJECT_BODY, &body.into_bytes());
    Ok(())
}

/// `RPOINT_SIZE`, `ENVMOD_SIZE`, the Player Start's box, `PSOBJECT_SIZE`, `VIS_RADIUS`.
pub const RPOINT_SIZE: f32 = 0.5;
pub const ENVMOD_SIZE: f32 = 0.25;
pub const PLAYERSTART_HALF_WIDTH: f32 = 0.4;
pub const PLAYERSTART_HEIGHT: f32 = 1.8;
pub const PSOBJECT_SIZE: f32 = 0.5;
pub const VIS_RADIUS: f32 = 0.25;

fn rpoint_box(p: Vec3) -> Aabb {
    Aabb::new(
        Vec3::new(p.x - RPOINT_SIZE, p.y, p.z - RPOINT_SIZE),
        Vec3::new(p.x + RPOINT_SIZE, p.y + RPOINT_SIZE * 2.0, p.z + RPOINT_SIZE),
    )
}

/// `GetBox`: the world box. `local_ref` gives a library object's local box by reference name.
#[must_use]
pub fn world_box(state: &SceneState, obj: &Object, local_ref: &dyn Fn(&str) -> Option<Aabb>) -> Option<Aabb> {
    let p = obj.position();
    match &obj.data {
        ObjData::SceneObject(d) => local_ref(&d.reference).map(|b| xform_box(&b, &obj.world())),
        ObjData::Light(d) => Some(point_box(p, d.range)),
        ObjData::Glow(d) => Some(point_box(p, d.radius)),
        ObjData::Shape(d) => d.world_box(&obj.xform),
        ObjData::SoundEnv(d) => d.shape.world_box(&obj.xform),
        ObjData::FogVolume(d) => d.shape.world_box(&obj.xform),
        ObjData::SoundSrc(d) => Some(point_box(p, d.max_distance)),
        ObjData::Spawn(d) => {
            let mut b = match d.kind {
                PointKind::RPoint => rpoint_box(p),
                PointKind::EnvMod => {
                    point_box(p, if state.is_selected(obj.id) { d.env.radius } else { ENVMOD_SIZE })
                }
                PointKind::PlayerStart => Aabb::new(
                    Vec3::new(p.x - PLAYERSTART_HALF_WIDTH, p.y, p.z - PLAYERSTART_HALF_WIDTH),
                    Vec3::new(
                        p.x + PLAYERSTART_HALF_WIDTH,
                        p.y + PLAYERSTART_HEIGHT,
                        p.z + PLAYERSTART_HALF_WIDTH,
                    ),
                ),
                PointKind::Spawn => match d.attached.as_ref().and_then(|a| attached_box(a)) {
                    Some(sb) => sb,
                    None => rpoint_box(p),
                },
            };
            if let Some(a) = &d.attached
                && let Some(ab) = attached_world_box(a)
            {
                b.merge(&ab);
            }
            Some(b)
        }
        ObjData::Way(d) => Some(d.bounds()),
        ObjData::Portal(d) => d.bounds(),
        ObjData::Sector(d) => {
            let mut b = Aabb::INVALID;
            for (o, _) in &d.items {
                if let Some(item) = state.find(ObjClass::SceneObject, o)
                    && let Some(ib) = world_box(state, item, local_ref)
                {
                    b.merge(&ib);
                }
            }
            b.is_valid().then_some(b)
        }
        ObjData::Group(g) => {
            let mut b = Aabb::INVALID;
            for id in &g.members {
                let Some(m) = state.get(*id) else { continue };
                match m.class {
                    ObjClass::SpawnPoint | ObjClass::SceneObject | ObjClass::Way => {
                        if let Some(mb) = world_box(state, m, local_ref) {
                            b.merge(&mb);
                        }
                    }
                    _ => b.modify(m.position()),
                }
            }
            if !b.is_valid() {
                b = point_box(p, 0.5);
            }
            Some(b)
        }
        ObjData::Particles(_) => Some(point_box(p, PSOBJECT_SIZE)),
        ObjData::Kept(_) => Some(point_box(p, VIS_RADIUS)),
    }
}

/// A spawn's box from its shape: the shapes' extents carried by the shape's transform.
fn attached_box(a: &Object) -> Option<Aabb> {
    let ObjData::Shape(s) = &a.data else { return None };
    if s.shapes.is_empty() {
        return None;
    }
    let m = a.xform.matrix();
    let mut b = Aabb::INVALID;
    for sh in &s.shapes {
        if sh.kind == CF_SPHERE {
            b.modify(m.transform_tiny(sh.center() + Vec3::splat(sh.radius())));
            b.modify(m.transform_tiny(sh.center() - Vec3::splat(sh.radius())));
        } else {
            let bm = sh.matrix();
            b.modify(m.transform_tiny(bm.transform_tiny(Vec3::splat(0.5))));
            b.modify(m.transform_tiny(bm.transform_tiny(Vec3::splat(-0.5))));
        }
    }
    Some(b)
}

fn attached_world_box(a: &Object) -> Option<Aabb> {
    match &a.data {
        ObjData::Shape(s) => s.world_box(&a.xform),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(class: ObjClass) -> Object {
        let mut o = Object::new(class, &format!("{}_1", class.name()), default_data(class));
        o.xform.position = Vec3::new(1.0, 2.0, 3.0);
        o.marks.authored = true;
        if let ObjData::Light(l) = &mut o.data {
            l.flags |= light_flags::POINT_FUZZY;
            l.fuzzy = Some(Fuzzy::default());
            l.anim_ref = "fire".into();
            // `%f` keeps six decimals: a value with more would not come back exactly
            l.cone = 0.5;
        }
        if let ObjData::SceneObject(s) = &mut o.data {
            s.reference = "props\\box_1".into();
            s.flags = SO_USE_SURFACE;
            s.surfaces.push(SurfaceOverride {
                name: "default".into(),
                texture: "a\\b".into(),
                ..SurfaceOverride::default()
            });
        }
        if let ObjData::Shape(s) = &mut o.data {
            s.shapes.push(ShapeDef::sphere(Vec3::ZERO, 2.0));
        }
        if let ObjData::Sector(s) = &mut o.data {
            s.items.push(("wall".into(), "mesh".into()));
        }
        if let ObjData::Portal(p) = &mut o.data {
            p.front = "a".into();
            p.back = "b".into();
            p.vertices = vec![Vec3::ZERO, Vec3::X, Vec3::Y];
        }
        if let ObjData::Particles(p) = &mut o.data {
            p.reference = "fire\\fire_01".into();
        }
        if let ObjData::Spawn(s) = &mut o.data {
            s.kind = PointKind::EnvMod;
            s.env.radius = 33.0;
        }
        o
    }

    /// Every class this crate writes by itself (groups need members: see `io`'s tests).
    fn classes() -> Vec<ObjClass> {
        ObjClass::ALL.iter().copied().filter(|c| c.is_object_tool() && *c != ObjClass::Group).collect()
    }

    #[test]
    fn every_class_round_trips_through_ltx() {
        let state = SceneState::default();
        for class in classes() {
            let o = sample(class);
            let mut w = IniWriter::new();
            write_ltx(&mut w, "object_0", &o, &state, 0).unwrap();
            let r = IniReader::parse(&w.save(), "t").unwrap();
            let mut cx = ReadCx::new(None);
            let back = read_ltx(&r, "object_0", &mut cx).unwrap().obj;
            assert_eq!(back.class, class);
            assert_eq!(back.name, o.name);
            assert_eq!(back.marks, o.marks);
            if class == ObjClass::SoundEnv {
                // oracle: the environments are not read back from LTX
                continue;
            }
            assert_eq!(back.data, o.data, "{class:?}");
        }
    }

    #[test]
    fn every_class_round_trips_through_the_stream() {
        let state = SceneState::default();
        for class in classes() {
            let o = sample(class);
            let mut w = Writer::new();
            write_stream(&mut w, &o, &state, 0).unwrap();
            let bytes = w.into_bytes();
            let mut cx = ReadCx::new(None);
            let back = read_stream(&Reader::new(&bytes), &mut cx).unwrap().obj;
            assert_eq!(back.data, o.data, "{class:?}");
            assert_eq!(back.xform, o.xform);
            // and writes the same bytes again
            let mut w2 = Writer::new();
            write_stream(&mut w2, &back, &state, 0).unwrap();
            assert_eq!(w2.into_bytes(), bytes, "{class:?}");
        }
    }

    #[test]
    fn a_refused_version_is_kept_and_written_back() {
        let mut w = IniWriter::new();
        w.w_u32("object_0", "clsid", ObjClass::Light.id());
        w.w_u32("object_0", "co_flags", 0);
        w.w_string("object_0", "name", "old");
        w.w_fvector3("object_0", "position", Vec3::ZERO);
        w.w_fvector3("object_0", "rotation", Vec3::ZERO);
        w.w_fvector3("object_0", "scale", Vec3::ONE);
        w.w_u32("object_0", "version", 0x99);
        w.w_string("object_0", "future_key", "42");
        let text = w.save();
        let r = IniReader::parse(&text, "t").unwrap();
        let mut cx = ReadCx::new(None);
        let back = read_ltx(&r, "object_0", &mut cx).unwrap().obj;
        assert!(matches!(back.data, ObjData::Kept(_)));
        assert_eq!(cx.warnings.len(), 1);
        let mut w2 = IniWriter::new();
        write_ltx(&mut w2, "object_0", &back, &SceneState::default(), 0).unwrap();
        assert_eq!(w2.save(), text);
    }
}
