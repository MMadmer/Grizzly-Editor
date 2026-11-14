//! The X-Ray SDK scene importer's readers of the sixteen kinds: an object of an LTX part
//! (`[object_N]` sections) or of a binary stream (`CHUNK_OBJECT_CLASS` / `CHUNK_OBJECT_BODY`),
//! into the scene's model. An object of a version the importer does not read is kept
//! ([`KeptObject`]) and named in the import's warnings.

use std::sync::Arc;

use gz_base::{Blob, SmolStr};
use gz_chunk::Reader;
use gz_entities::{Entity, Settings};

use super::ltx::IniReader;
use crate::SceneError;
use crate::class::ObjClass;
use crate::math::{Transform, Vec3};
use crate::model::{MotionRaw, ObjData, Object, co};
use crate::objects::*;

pub const SCENEOBJ_VERSION: u32 = 0x12;
pub const LIGHT_VERSION: u32 = 0x11;
pub const GLOW_VERSION: u32 = 0x12;
pub const SOUND_SOURCE_VERSION: u32 = 0x14;
pub const PS_VERSION: u32 = 0x13;
pub const SHAPE_VERSION: u32 = 2;
pub const SOUND_ENV_VERSION: u32 = 0x12;
pub const FOG_VOL_VERSION: u32 = 2;
pub const WAYOBJECT_VERSION: u32 = 0x13;
pub const SECTOR_VERSION: u32 = 0x12;
pub const PORTAL_VERSION: u32 = 0x10;
pub const SPAWNPOINT_VERSION: u32 = 0x17;
pub const XMS_ORDER_VERSION: u8 = 1;
pub const GROUP_VERSION: u32 = 0x12;
pub const CHUNK_OBJECT_CLASS: u32 = 0x7703;
pub const CHUNK_OBJECT_BODY: u32 = 0x7777;
const CO_CHUNK_TRANSFORM: u32 = 0xF903;
const CO_CHUNK_MOTION: u32 = 0xF905;
const CO_CHUNK_FLAGS: u32 = 0xF906;
const CO_CHUNK_NAME: u32 = 0xF907;
const CO_CHUNK_MOTION_PARAM: u32 = 0xF908;

/// What a class body read says: understood, or of a version the importer does not read.
pub enum Body<T> {
    Read(T),
    Refused(String),
}

fn chunk_u16(r: &Reader<'_>, id: u32) -> Result<Option<u16>, SceneError> {
    Ok(match r.open_chunk(id)? {
        Some(c) => Some(c.reader().r_u16()?),
        None => None,
    })
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

impl SceneObjectData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Self, SceneError> {
        let reference = r.r_string(s, "reference_name")?;
        let flags = r.opt_u32(s, "flags").unwrap_or(0);
        let mut surfaces = Vec::new();
        if flags & SO_USE_SURFACE != 0 && r.line_exist(s, "000001") {
            let mut n = 1u32;
            let mut next = |r: &IniReader| -> Result<String, SceneError> {
                let k = format!("{n:06}");
                n += 1;
                r.r_string_wb(s, &k)
            };
            let count: u32 = next(r)?.trim().parse().unwrap_or(0);
            for _ in 0..count.min(4096) {
                surfaces.push(SurfaceOverride {
                    name: next(r)?.into(),
                    shader: next(r)?.into(),
                    shader_xrlc: next(r)?.into(),
                    game_mtl: next(r)?.into(),
                    texture: next(r)?.into(),
                    vmap: next(r)?.into(),
                });
            }
        }
        Ok(Self { reference: reference.into(), flags, surfaces })
    }

    /// `LoadStream`; version `0x10` carries the placement in a chunk of its own, which
    /// `placement` receives.
    pub fn read_stream(r: &Reader<'_>, placement: &mut Option<[Vec3; 3]>) -> Result<Body<Self>, SceneError> {
        let Some(version) = chunk_u16(r, 0x0900)? else {
            return Ok(Body::Refused("CSceneObject: no version".into()));
        };
        if version == 0x10
            && let Some(c) = r.open_chunk(0x0904)?
        {
            let mut q = c.reader();
            *placement = Some([q.r_vec3()?, q.r_vec3()?, q.r_vec3()?]);
        }
        let Some(c) = r.open_chunk(0x0902)? else {
            return Ok(Body::Refused("CSceneObject: no reference".into()));
        };
        let mut q = c.reader();
        if version <= 0x11 {
            q.r_u32()?;
            q.r_u32()?;
        }
        let reference = q.r_stringz()?;
        let flags = match r.open_chunk(0x0905)? {
            Some(c) => c.reader().r_u32()?,
            None => 0,
        };
        let mut surfaces = Vec::new();
        if flags & SO_USE_SURFACE != 0
            && let Some(c) = r.open_chunk(0x0906)?
        {
            let mut q = c.reader();
            let n = q.r_u32()?;
            for _ in 0..n.min(4096) {
                surfaces.push(SurfaceOverride {
                    name: q.r_stringz()?.into(),
                    shader: q.r_stringz()?.into(),
                    shader_xrlc: q.r_stringz()?.into(),
                    game_mtl: q.r_stringz()?.into(),
                    texture: q.r_stringz()?.into(),
                    vmap: q.r_stringz()?.into(),
                });
            }
        }
        Ok(Body::Read(Self { reference: reference.into(), flags, surfaces }))
    }
}

impl LightData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        if r.r_u32(s, "version")? != LIGHT_VERSION {
            return Ok(Body::Refused("CLight: Unsupported version.".into()));
        }
        let flags = r.r_u32(s, "light_flags")?;
        let fuzzy = if flags & light_flags::POINT_FUZZY != 0 {
            let n = r.r_u32(s, "fuzzy_point_count")?;
            let mut points = Vec::new();
            for i in 0..n.min(1 << 16) {
                points.push(r.r_fvector3(s, &format!("fuzzy_point_{i}"))?);
            }
            Some(Fuzzy {
                shape: r.r_u8(s, "fuzzy_shape_type")?,
                sphere_radius: r.r_float(s, "fuzzy_sphere_radius")?,
                box_dim: r.r_fvector3(s, "fuzzy_box_dim")?,
                points,
            })
        } else {
            None
        };
        Ok(Body::Read(Self {
            kind: r.r_u32(s, "type")?,
            color: r.r_fcolor(s, "color")?,
            brightness: r.r_float(s, "brightness")?,
            range: r.r_float(s, "range")?,
            attenuation: [
                r.r_float(s, "attenuation0")?,
                r.r_float(s, "attenuation1")?,
                r.r_float(s, "attenuation2")?,
            ],
            cone: r.r_float(s, "cone")?,
            virtual_size: r.r_float(s, "virtual_size")?,
            flags,
            lcontrol: r.r_u32(s, "light_control")?,
            anim_ref: r.r_string(s, "anim_ref_name")?.into(),
            falloff_tex: r.r_string(s, "fallof_texture")?.into(),
            fuzzy,
        }))
    }

    /// `LoadStream`; the old `Flight` chunk carries the position, which `position` receives.
    pub fn read_stream(r: &Reader<'_>, position: &mut Option<Vec3>) -> Result<Body<Self>, SceneError> {
        let version = chunk_u16(r, 0xB411)?.unwrap_or(0);
        if version != 0x10 && version != LIGHT_VERSION as u16 {
            return Ok(Body::Refused("CLight: Unsupported version.".into()));
        }
        let mut d = Self::default();
        if let Some(c) = r.open_chunk(0xB442)? {
            let mut q = c.reader();
            d.kind = q.r_u32()?;
            d.color = [q.r_f32()?, q.r_f32()?, q.r_f32()?, q.r_f32()?];
            d.brightness = q.r_f32()?;
            d.range = q.r_f32()?;
            d.attenuation = [q.r_f32()?, q.r_f32()?, q.r_f32()?];
            d.cone = q.r_f32()?;
            d.virtual_size = q.r_f32()?;
        } else {
            let Some(c) = r.open_chunk(0xB435)? else {
                return Ok(Body::Refused("CLight: no parameters".into()));
            };
            let mut q = c.reader();
            d.kind = q.r_u32()?;
            d.color = [q.r_f32()?, q.r_f32()?, q.r_f32()?, q.r_f32()?];
            q.advance(32)?;
            *position = Some(q.r_vec3()?);
            q.r_vec3()?;
            d.range = q.r_f32()?;
            q.r_f32()?;
            d.attenuation = [q.r_f32()?, q.r_f32()?, q.r_f32()?];
            q.r_f32()?;
            d.cone = q.r_f32()?;
            if let Some(b) = r.open_chunk(0xB425)? {
                d.brightness = b.reader().r_f32()?;
            }
        }
        if let Some(c) = r.open_chunk(0xB413)? {
            d.flags = c.reader().r_u32()?;
        }
        if let Some(c) = r.open_chunk(0xB441)? {
            d.lcontrol = c.reader().r_u32()?;
        }
        if d.kind == LT_DIRECT {
            return Ok(Body::Refused("CLight: Can't load sun.".into()));
        }
        if let Some(c) = r.open_chunk(0xB438)? {
            d.anim_ref = c.reader().r_stringz()?.into();
        }
        if let Some(c) = r.open_chunk(0xB439)? {
            d.falloff_tex = c.reader().r_stringz()?.into();
        }
        if let Some(c) = r.open_chunk(0xB440)? {
            let mut q = c.reader();
            let shape = q.r_u8()?;
            let sphere_radius = q.r_f32()?;
            let box_dim = q.r_vec3()?;
            let n = usize::try_from(q.r_i16()?.max(0)).unwrap_or(0);
            if n * 12 > q.len() {
                return Err(SceneError::load("CLight: the fuzzy points are cut short"));
            }
            let mut points = Vec::with_capacity(n);
            for _ in 0..n {
                points.push(q.r_vec3()?);
            }
            d.fuzzy = Some(Fuzzy { shape, sphere_radius, box_dim, points });
            d.flags |= light_flags::POINT_FUZZY;
        } else {
            d.flags &= !light_flags::POINT_FUZZY;
        }
        Ok(Body::Read(d))
    }
}

impl GlowData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        if r.r_u32(s, "version")? != GLOW_VERSION {
            return Ok(Body::Refused("CGlow: Unsupported version.".into()));
        }
        Ok(Body::Read(Self {
            shader: r.r_string(s, "shader_name")?.into(),
            texture: r.r_string(s, "texture_name")?.into(),
            radius: r.r_float(s, "radius")?,
            flags: r.r_u32(s, "flags")? as u16,
        }))
    }

    pub fn read_stream(r: &Reader<'_>, position: &mut Option<Vec3>) -> Result<Body<Self>, SceneError> {
        let version = chunk_u16(r, 0xC411)?.unwrap_or(0);
        if version != 0x11 && version != GLOW_VERSION as u16 {
            return Ok(Body::Refused("CGlow: Unsupported version.".into()));
        }
        let mut d = Self::default();
        if let Some(c) = r.open_chunk(0xC414)? {
            d.shader = c.reader().r_stringz()?.into();
        }
        let (Some(t), Some(p)) = (r.open_chunk(0xC415)?, r.open_chunk(0xC413)?) else {
            return Ok(Body::Refused("CGlow: the texture or the parameters are missing".into()));
        };
        d.texture = t.reader().r_stringz()?.into();
        let mut q = p.reader();
        d.radius = q.r_f32()?;
        if version == 0x11 {
            *position = Some(q.r_vec3()?);
        }
        if let Some(c) = r.open_chunk(0xC416)? {
            d.flags = c.reader().r_u16()?;
        }
        Ok(Body::Read(d))
    }
}

impl SoundSrcData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        if r.r_u32(s, "version")? != SOUND_SOURCE_VERSION {
            return Ok(Body::Refused("ESoundSource: Unsupported version.".into()));
        }
        Ok(Body::Read(Self {
            kind: r.r_u32(s, "snd_type")?,
            wav: r.r_string(s, "snd_name")?.into(),
            flags: r.r_u32(s, "flags")?,
            position: r.r_fvector3(s, "snd_position")?,
            volume: r.r_float(s, "volume")?,
            freq: r.r_float(s, "freq")?,
            min_distance: r.r_float(s, "min_dist")?,
            max_distance: r.r_float(s, "max_dist")?,
            max_ai_distance: r.r_float(s, "max_ai_dist")?,
            random_pause: r.r_fvector2(s, "random_pause")?,
            active_time: r.r_fvector2(s, "active_time")?,
            play_time: r.r_fvector2(s, "play_time")?,
        }))
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        if chunk_u16(r, 0x1001)? != Some(SOUND_SOURCE_VERSION as u16) {
            return Ok(Body::Refused("ESoundSource: Unsupported version.".into()));
        }
        let mut d = Self::default();
        let (Some(t), Some(n)) = (r.open_chunk(0x1002)?, r.open_chunk(0x1003)?) else {
            return Ok(Body::Refused("ESoundSource: the type or the name is missing".into()));
        };
        d.kind = t.reader().r_u32()?;
        d.wav = n.reader().r_stringz()?.into();
        let params = match (r.open_chunk(0x1007)?, r.open_chunk(0x1006)?, r.open_chunk(0x1004)?) {
            (Some(c), _, _) | (None, Some(c), _) => Some((c, true)),
            (None, None, Some(c)) => Some((c, false)),
            _ => None,
        };
        let Some((c, ai)) = params else {
            return Ok(Body::Refused("ESoundSource: Can't load Sound Source. Unsupported version.".into()));
        };
        let mut q = c.reader();
        d.position = q.r_vec3()?;
        d.volume = q.r_f32()?;
        d.freq = q.r_f32()?;
        d.min_distance = q.r_f32()?;
        d.max_distance = q.r_f32()?;
        d.max_ai_distance = if ai { q.r_f32()? } else { d.max_distance };
        if let Some(c) = r.open_chunk(0x1005)? {
            d.flags = c.reader().r_u32()?;
        }
        if let Some(c) = r.open_chunk(0x1008)? {
            let mut q = c.reader();
            d.random_pause = [q.r_f32()?, q.r_f32()?];
            d.active_time = [q.r_f32()?, q.r_f32()?];
            d.play_time = [q.r_f32()?, q.r_f32()?];
        }
        if d.kind != 0 {
            return Ok(Body::Refused("ESoundSource: unknown type".into()));
        }
        Ok(Body::Read(d))
    }
}

impl ParticlesData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Self, SceneError> {
        let version = r.r_u32(s, "version")?;
        let mut d = Self { reference: r.r_string(s, "ref_name")?.into(), ..Self::default() };
        if version >= 0x12 {
            d.game_type = r.r_u16(s, "game_type")?;
        }
        Ok(d)
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        let version = chunk_u16(r, 1)?.unwrap_or(0);
        if version < 0x11 {
            return Ok(Body::Refused("PSObject: Unsupported version.".into()));
        }
        let Some(c) = r.open_chunk(2)? else {
            return Ok(Body::Refused("PSObject: no reference".into()));
        };
        let mut d = Self { reference: c.reader().r_stringz()?.into(), ..Self::default() };
        if version >= 0x13 {
            if let Some(c) = r.open_chunk(4)? {
                d.game_type = c.reader().r_u16()?;
            } else {
                return Ok(Body::Refused("PSObject: no game type".into()));
            }
        }
        Ok(Body::Read(d))
    }
}

impl ShapeData {
    /// The `shape_*` keys (`CEditShape::LoadLTX` after the common part).
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Self, SceneError> {
        let version = r.r_u32(s, "version")?;
        let count = r.r_u32(s, "shapes_count")?;
        let usage = if version > 1 { r.r_u8(s, "shape_type")? } else { SHAPE_COMMON };
        let mut shapes = Vec::new();
        for i in 0..count.min(1 << 16) {
            let kind = r.r_u8(s, &format!("shape_type_{i}"))?;
            if kind == CF_SPHERE {
                let c = r.r_fvector3(s, &format!("shape_center_{i}"))?;
                let rad = r.r_float(s, &format!("shape_radius_{i}"))?;
                shapes.push(ShapeDef::sphere(c, rad));
            } else {
                let mut d = ShapeDef::boxed(&crate::math::Mat4::IDENTITY);
                d.kind = kind;
                for (row, key) in ["i", "j", "k", "c"].iter().enumerate() {
                    d.set_row(row, r.r_fvector3(s, &format!("shape_matrix_{key}_{i}"))?);
                }
                shapes.push(d);
            }
        }
        Ok(Self { shapes, usage })
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        if r.open_chunk(0)?.is_none() {
            return Ok(Body::Refused("CEditShape: no version".into()));
        }
        let Some(c) = r.open_chunk(1)? else {
            return Ok(Body::Refused("CEditShape: no shapes".into()));
        };
        let mut q = c.reader();
        let n = q.r_u32()? as usize;
        if n.saturating_mul(68) > q.len() {
            return Err(SceneError::load("CEditShape: the shape list is cut short"));
        }
        let mut shapes = Vec::with_capacity(n);
        for _ in 0..n {
            let kind = q.r_u8()?;
            let pad = [q.r_u8()?, q.r_u8()?, q.r_u8()?];
            let mut data = [0.0; 16];
            for d in &mut data {
                *d = q.r_f32()?;
            }
            shapes.push(ShapeDef { kind, pad, data });
        }
        let usage = match r.open_chunk(2)? {
            Some(c) => c.reader().r_u8()?,
            None => SHAPE_COMMON,
        };
        Ok(Body::Read(Self { shapes, usage }))
    }
}

impl SoundEnvData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        if r.r_u32(s, "version")? != SOUND_ENV_VERSION {
            return Ok(Body::Refused("ESoundSource: Unsupported version.".into()));
        }
        // oracle: the C++ LTX reader never reads env_inner / env_outer back, so an LTX scene
        // loses them on load; kept that way so both editors answer the same.
        Ok(Body::Read(Self {
            shape: ShapeData::read_ltx(r, s)?,
            inner: SmolStr::default(),
            outer: SmolStr::default(),
        }))
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        let v = match r.open_chunk(0x1001)? {
            Some(c) => c.reader().r_u16()?,
            None => 0,
        };
        if u32::from(v) != SOUND_ENV_VERSION {
            return Ok(Body::Refused("ESoundSource: Unsupported version.".into()));
        }
        let shape = match ShapeData::read_stream(r)? {
            Body::Read(s) => s,
            Body::Refused(w) => return Ok(Body::Refused(w)),
        };
        let Some(c) = r.open_chunk(0x1003)? else {
            return Ok(Body::Refused("ESoundEnvironment: no environments".into()));
        };
        let mut q = c.reader();
        Ok(Body::Read(Self { shape, inner: q.r_stringz()?.into(), outer: q.r_stringz()?.into() }))
    }
}

impl FogData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Self, SceneError> {
        let version = r.r_u32(s, "version")?;
        let mut d = Self::default();
        if version > 0 {
            d.kind = r.r_u8(s, "folume_type")?;
            d.group_id = r.r_u32(s, "group_id")?;
        }
        if version > 1 && d.kind == FV_EMITTER {
            d.profile = r.r_string(s, "profile")?.into();
        }
        Ok(d)
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        let Some(v) = r.open_chunk(0x1002)? else {
            return Ok(Body::Refused("EFogVolume: no version".into()));
        };
        let version = v.reader().r_u16()?;
        let mut d = Self::default();
        if let Some(c) = r.open_chunk(0x1003)? {
            let mut q = c.reader();
            d.kind = q.r_u8()?;
            d.group_id = q.r_u32()?;
            if version > 1 && d.kind == FV_EMITTER {
                d.profile = q.r_stringz()?.into();
            }
        }
        Ok(Body::Read(d))
    }
}

impl WayData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        if r.r_u32(s, "version")? != WAYOBJECT_VERSION {
            return Ok(Body::Refused("CWayPoint: Unsupported version.".into()));
        }
        let n = r.r_u32(s, "wp_count")?;
        let mut w = Self { kind: 0, points: Vec::new() };
        for i in 0..n.min(1 << 16) {
            w.points.push(WayPoint {
                position: r.r_fvector3(s, &format!("wp_{i}_pos"))?,
                flags: r.r_u32(s, &format!("wp_{i}_flags"))?,
                selected: r.r_bool(s, &format!("wp_{i}_selected"))?,
                name: r.r_string(s, &format!("wp_{i}_name"))?.into(),
                links: Vec::new(),
            });
        }
        // oracle: the links come back in the file's key order (`link_wp_0_10` before
        // `link_wp_0_2`), which is the order they are created in again
        for (key, _) in r.lines(s) {
            let Some(rest) = key.strip_prefix("link_wp_") else { continue };
            let mut it = rest.split('_');
            let (Some(a), Some(b)) = (it.next(), it.next()) else {
                return Err(SceneError::load(format!("bad waypoint link record format: [{s}] {key}")));
            };
            let (Ok(from), Ok(_)) = (a.parse::<usize>(), b.parse::<usize>()) else {
                return Err(SceneError::load(format!("bad waypoint link record format: [{s}] {key}")));
            };
            let v = r.r_fvector2(s, &key)?;
            w.create_link(from, v[0].floor().max(0.0) as usize, v[1]);
        }
        w.kind = r.r_u32(s, "type")?;
        Ok(Body::Read(w))
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        let version = match r.open_chunk(1)? {
            Some(c) => c.reader().r_u16()?,
            None => return Ok(Body::Refused("CWayPoint: no version".into())),
        };
        if u32::from(version) != WAYOBJECT_VERSION {
            return Ok(Body::Refused("CWayPoint: Unsupported version.".into()));
        }
        let (Some(pc), Some(lc), Some(tc)) = (r.open_chunk(2)?, r.open_chunk(3)?, r.open_chunk(4)?) else {
            return Ok(Body::Refused("CWayPoint: a chunk is missing".into()));
        };
        let mut q = pc.reader();
        let n = q.r_u16()? as usize;
        let mut w = Self { kind: 0, points: Vec::with_capacity(n.min(q.len() / 19 + 1)) };
        for _ in 0..n {
            let position = q.r_vec3()?;
            let flags = q.r_u32()?;
            let selected = q.r_u16()? != 0;
            let name = q.r_stringz()?;
            w.points.push(WayPoint { name: name.into(), position, flags, selected, links: Vec::new() });
        }
        let mut q = lc.reader();
        let ln = q.r_u16()?;
        for _ in 0..ln {
            let from = q.r_u16()? as usize;
            let to = q.r_u16()? as usize;
            let prob = q.r_f32()?;
            if from >= w.points.len() || to >= w.points.len() {
                return Err(SceneError::load("CWayPoint: a link names a point that does not exist"));
            }
            w.create_link(from, to, prob);
        }
        w.kind = tc.reader().r_u32()?;
        Ok(Body::Read(w))
    }
}

impl SectorData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        let version = r.r_u32(s, "version")?;
        if version < 0x11 {
            return Ok(Body::Refused("CSector: Unsupported version.".into()));
        }
        let mut d = Self {
            color: fcolor_from(r.r_color(s, "sector_color")?),
            default: r.r_bool(s, "default")?,
            ..Self::default()
        };
        let n = r.r_u32(s, "items_count")?;
        for i in 0..n.min(1 << 16) {
            d.items.push((
                r.r_string(s, &format!("item_object_name_{i:04}"))?.into(),
                r.r_string(s, &format!("item_mesh_name_{i:04}"))?.into(),
            ));
        }
        if version >= 0x12 {
            d.map_idx = r.r_u8(s, "change_map_to_idx")?;
        }
        Ok(Body::Read(d))
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        let version = match r.open_chunk(0xF010)? {
            Some(c) => c.reader().r_u16()?,
            None => 0,
        };
        if u32::from(version) != SECTOR_VERSION {
            return Ok(Body::Refused("CSector: Unsupported version.".into()));
        }
        let (Some(cc), Some(pc)) = (r.open_chunk(0xF020)?, r.open_chunk(0xF025)?) else {
            return Ok(Body::Refused("CSector: a chunk is missing".into()));
        };
        let mut q = cc.reader();
        let mut d = Self { color: [q.r_f32()?, q.r_f32()?, q.r_f32()?, q.r_f32()?], ..Self::default() };
        d.default = pc.reader().r_u8()? != 0;
        if let Some(items) = r.open_chunk(0xF030)? {
            for (_, _, body) in items.reader().chunks() {
                if let Some(one) = Reader::new(body).open_chunk(0xF031)? {
                    let mut q = one.reader();
                    d.items.push((q.r_stringz()?.into(), q.r_stringz()?.into()));
                }
            }
        }
        if let Some(c) = r.open_chunk(0xF032)? {
            d.map_idx = c.reader().r_u8()?;
        }
        Ok(Body::Read(d))
    }
}

impl PortalData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        if r.r_u32(s, "version")? != PORTAL_VERSION {
            return Ok(Body::Refused("CPortal: Unsupported version.".into()));
        }
        let mut d = Self {
            front: r.r_string(s, "sector_front")?.into(),
            back: r.r_string(s, "sector_back")?.into(),
            vertices: Vec::new(),
        };
        let n = r.r_u32(s, "vert_count")?;
        for i in 0..n.min(1 << 16) {
            d.vertices.push(r.r_fvector3(s, &format!("vertex_{i:04}"))?);
        }
        Ok(Body::Read(d))
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        let version = match r.open_chunk(0xFA10)? {
            Some(c) => c.reader().r_u16()?,
            None => 0,
        };
        if u32::from(version) != PORTAL_VERSION {
            return Ok(Body::Refused("CPortal: Unsupported version.".into()));
        }
        let mut d = Self::default();
        if let Some(c) = r.open_chunk(0xFA30)? {
            d.front = c.reader().r_stringz()?.into();
        }
        if let Some(c) = r.open_chunk(0xFA40)? {
            d.back = c.reader().r_stringz()?.into();
        }
        let Some(c) = r.open_chunk(0xFA50)? else {
            return Ok(Body::Refused("CPortal: no vertices".into()));
        };
        let mut q = c.reader();
        let n = q.r_u16()? as usize;
        if n * 12 > q.len() {
            return Err(SceneError::load("CPortal: the vertices are cut short"));
        }
        for _ in 0..n {
            d.vertices.push(q.r_vec3()?);
        }
        Ok(Body::Read(d))
    }
}

impl SpawnData {
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
}
