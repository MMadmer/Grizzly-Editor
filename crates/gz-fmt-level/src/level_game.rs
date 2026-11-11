//! The small game files of a built level: `level.game` (way objects and respawn points),
//! `level.env_mod`, `level.snd_static`, `level.ps_static` and `level.fog_vol`.

use glam::Vec3;
use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::{LevelError, Result};

/// The chunk of way objects in `level.game` (`WAY_PATROLPATH_CHUNK`).
pub const WAY_CHUNK: u32 = 0x1000;
/// The chunk of respawn points in `level.game` (`RPOINT_CHUNK`).
pub const RPOINT_CHUNK: u32 = 0x2000;
/// `WAYOBJECT_VERSION`.
pub const WAYOBJECT_VERSION: u16 = 0x0013;
/// `SPAWNPOINT_VERSION`, the version `level.env_mod` starts with.
pub const SPAWNPOINT_VERSION: u32 = 0x0017;

const WAY_CHUNK_VERSION: u32 = 0x0001;
const WAY_CHUNK_POINTS: u32 = 0x0002;
const WAY_CHUNK_LINKS: u32 = 0x0003;
const WAY_CHUNK_NAME: u32 = 0x0005;

/// A point of a way object.
#[derive(Clone, Debug, PartialEq)]
pub struct WayPoint {
    pub pos: Vec3,
    pub flags: u32,
    pub name: SmolStr,
}

/// A link of a way object, by point index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WayLink {
    pub from: u16,
    pub to: u16,
    pub probability: f32,
}

/// A way object (a patrol path as the level build writes it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WayObject {
    pub name: SmolStr,
    pub points: Vec<WayPoint>,
    pub links: Vec<WayLink>,
}

impl WayObject {
    fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let version = r.need_chunk(WAY_CHUNK_VERSION)?.reader().r_u16()?;
        if version != WAYOBJECT_VERSION {
            return Err(LevelError::Version { what: "way object", found: u32::from(version) });
        }
        let name = SmolStr::new(r.need_chunk(WAY_CHUNK_NAME)?.reader().r_stringz()?);

        let chunk = r.need_chunk(WAY_CHUNK_POINTS)?;
        let mut c = chunk.reader();
        let count = c.r_u16()?;
        // a point is 17 bytes at least: the count cannot ask for more than the chunk holds
        let mut points = Vec::with_capacity(usize::from(count).min(c.elapsed() / 17));
        for _ in 0..count {
            points.push(WayPoint { pos: c.r_vec3()?, flags: c.r_u32()?, name: SmolStr::new(c.r_stringz()?) });
        }

        let chunk = r.need_chunk(WAY_CHUNK_LINKS)?;
        let mut c = chunk.reader();
        let count = c.r_u16()?;
        let mut links = Vec::with_capacity(usize::from(count).min(c.elapsed() / 8));
        for _ in 0..count {
            links.push(WayLink { from: c.r_u16()?, to: c.r_u16()?, probability: c.r_f32()? });
        }
        Ok(Self { name, points, links })
    }

    fn write(&self, w: &mut Writer) {
        w.chunk(WAY_CHUNK_VERSION, |w| w.w_u16(WAYOBJECT_VERSION));
        w.chunk(WAY_CHUNK_NAME, |w| w.w_stringz(&self.name));
        w.chunk(WAY_CHUNK_POINTS, |w| {
            w.w_u16(self.points.len() as u16);
            for p in &self.points {
                w.w_vec3(p.pos);
                w.w_u32(p.flags);
                w.w_stringz(&p.name);
            }
        });
        w.chunk(WAY_CHUNK_LINKS, |w| {
            w.w_u16(self.links.len() as u16);
            for l in &self.links {
                w.w_u16(l.from);
                w.w_u16(l.to);
                w.w_f32(l.probability);
            }
        });
    }
}

/// A respawn point.
#[derive(Clone, Debug, PartialEq)]
pub struct RPoint {
    pub pos: Vec3,
    pub rot: Vec3,
    pub team: u8,
    /// `ERPpointType`: 0 actor, 1 artefact, 2 item.
    pub kind: u8,
    /// The game types the point serves (reserved and 0 in old files).
    pub game_type: u16,
    /// The respawn items profile. `None` in files that predate it; the editor always writes one.
    pub profile: Option<SmolStr>,
}

/// `level.game`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LevelGame {
    pub ways: Vec<WayObject>,
    pub rpoints: Vec<RPoint>,
}

impl LevelGame {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let mut out = Self::default();
        if let Some(ways) = r.open_chunk(WAY_CHUNK)? {
            for (_, packed, body) in ways.reader().chunks() {
                if packed {
                    return Err(LevelError::Invalid("level.game: a way object is compressed".into()));
                }
                out.ways.push(WayObject::read(body)?);
            }
        }
        if let Some(points) = r.open_chunk(RPOINT_CHUNK)? {
            for (_, packed, body) in points.reader().chunks() {
                if packed {
                    return Err(LevelError::Invalid("level.game: a respawn point is compressed".into()));
                }
                let mut c = Reader::new(body);
                let mut p = RPoint {
                    pos: c.r_vec3()?,
                    rot: c.r_vec3()?,
                    team: c.r_u8()?,
                    kind: c.r_u8()?,
                    game_type: c.r_u16()?,
                    profile: None,
                };
                if !c.eof() {
                    p.profile = Some(SmolStr::new(c.r_stringz()?));
                }
                out.rpoints.push(p);
            }
        }
        Ok(out)
    }

    /// The file: both chunks, always, as the level build writes them.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.chunk(WAY_CHUNK, |w| {
            for (i, way) in self.ways.iter().enumerate() {
                w.chunk(i as u32, |w| way.write(w));
            }
        });
        w.chunk(RPOINT_CHUNK, |w| {
            for (i, p) in self.rpoints.iter().enumerate() {
                w.chunk(i as u32, |w| {
                    w.w_vec3(p.pos);
                    w.w_vec3(p.rot);
                    w.w_u8(p.team);
                    w.w_u8(p.kind);
                    w.w_u16(p.game_type);
                    if let Some(profile) = &p.profile {
                        w.w_stringz(profile);
                    }
                });
            }
        });
        w.into_bytes()
    }
}

/// An environment modifier of `level.env_mod`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvModifier {
    pub pos: Vec3,
    pub radius: f32,
    pub power: f32,
    pub view_dist: f32,
    pub fog_color: Vec3,
    pub fog_density: f32,
    pub ambient: Vec3,
    pub sky_color: Vec3,
    pub hemi_color: Vec3,
    /// Which of the values apply (`EEnvModUsedParams`). Files older than version `0x16` have
    /// none: every value applies.
    pub flags: u16,
}

/// `level.env_mod`: a version chunk, then one chunk per modifier.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvMods {
    /// `None` for a file without the version chunk (version `0x15`: no flags).
    pub version: Option<u32>,
    pub mods: Vec<EnvModifier>,
}

impl EnvMods {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let mut out = Self { version: None, mods: Vec::new() };
        for (i, (id, packed, body)) in Reader::new(bytes).chunks().enumerate() {
            if packed {
                return Err(LevelError::Invalid("level.env_mod: a chunk is compressed".into()));
            }
            let mut c = Reader::new(body);
            if i == 0 && id == 0 && body.len() == 4 {
                out.version = Some(c.r_u32()?);
                continue;
            }
            let with_flags = out.version.is_some_and(|v| v >= 0x16);
            out.mods.push(EnvModifier {
                pos: c.r_vec3()?,
                radius: c.r_f32()?,
                power: c.r_f32()?,
                view_dist: c.r_f32()?,
                fog_color: c.r_vec3()?,
                fog_density: c.r_f32()?,
                ambient: c.r_vec3()?,
                sky_color: c.r_vec3()?,
                hemi_color: c.r_vec3()?,
                flags: if with_flags { c.r_u16()? } else { u16::MAX },
            });
        }
        Ok(out)
    }

    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::new();
        let mut id = 0u32;
        if let Some(version) = self.version {
            w.chunk(id, |w| w.w_u32(version));
            id += 1;
        }
        let with_flags = self.version.is_some_and(|v| v >= 0x16);
        for m in &self.mods {
            w.chunk(id, |w| {
                w.w_vec3(m.pos);
                w.w_f32(m.radius);
                w.w_f32(m.power);
                w.w_f32(m.view_dist);
                w.w_vec3(m.fog_color);
                w.w_f32(m.fog_density);
                w.w_vec3(m.ambient);
                w.w_vec3(m.sky_color);
                w.w_vec3(m.hemi_color);
                if with_flags {
                    w.w_u16(m.flags);
                }
            });
            id += 1;
        }
        w.into_bytes()
    }
}

/// A static sound source of `level.snd_static`. Times are in milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticSound {
    pub wave: SmolStr,
    pub pos: Vec3,
    pub volume: f32,
    pub freq: f32,
    pub active_time: [u32; 2],
    pub play_time: [u32; 2],
    pub random_pause: [u32; 2],
}

/// Reads `level.snd_static`: one chunk per source, the data in a chunk 0 inside it.
pub fn read_static_sounds(bytes: &[u8]) -> Result<Vec<StaticSound>> {
    let mut out = Vec::new();
    for (_, packed, body) in Reader::new(bytes).chunks() {
        if packed {
            return Err(LevelError::Invalid("level.snd_static: a chunk is compressed".into()));
        }
        let inner = Reader::new(body).need_chunk(0)?;
        let mut c = inner.reader();
        out.push(StaticSound {
            wave: SmolStr::new(c.r_stringz()?),
            pos: c.r_vec3()?,
            volume: c.r_f32()?,
            freq: c.r_f32()?,
            active_time: [c.r_u32()?, c.r_u32()?],
            play_time: [c.r_u32()?, c.r_u32()?],
            random_pause: [c.r_u32()?, c.r_u32()?],
        });
    }
    Ok(out)
}

/// Writes `level.snd_static`.
#[must_use]
pub fn write_static_sounds(sounds: &[StaticSound]) -> Vec<u8> {
    let mut w = Writer::new();
    for (i, s) in sounds.iter().enumerate() {
        w.chunk(i as u32, |w| {
            w.chunk(0, |w| {
                w.w_stringz(&s.wave);
                w.w_vec3(s.pos);
                w.w_f32(s.volume);
                w.w_f32(s.freq);
                for v in s.active_time.into_iter().chain(s.play_time).chain(s.random_pause) {
                    w.w_u32(v);
                }
            });
        });
    }
    w.into_bytes()
}

/// A static particle system of `level.ps_static`.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticParticles {
    pub game_type: u16,
    pub reference: SmolStr,
    /// The transform, row after row.
    pub transform: [[f32; 4]; 4],
}

/// The version `level.ps_static` starts with.
pub const PS_STATIC_VERSION: u32 = 1;

/// Reads `level.ps_static`: a version chunk, then one chunk per system. A file from before the
/// version chunk has neither it nor the game type.
pub fn read_static_particles(bytes: &[u8]) -> Result<Vec<StaticParticles>> {
    let mut out = Vec::new();
    let mut versioned = false;
    for (i, (id, packed, body)) in Reader::new(bytes).chunks().enumerate() {
        if packed {
            return Err(LevelError::Invalid("level.ps_static: a chunk is compressed".into()));
        }
        let mut c = Reader::new(body);
        if i == 0 && id == 0 && body.len() == 4 {
            let version = c.r_u32()?;
            if version != PS_STATIC_VERSION {
                return Err(LevelError::Version { what: "level.ps_static", found: version });
            }
            versioned = true;
            continue;
        }
        let game_type = if versioned { c.r_u16()? } else { u16::MAX };
        out.push(StaticParticles {
            game_type,
            reference: SmolStr::new(c.r_stringz()?),
            transform: c.r_matrix()?,
        });
    }
    Ok(out)
}

/// Writes `level.ps_static`. Nothing at all for no systems: the build writes no file then.
#[must_use]
pub fn write_static_particles(systems: &[StaticParticles]) -> Vec<u8> {
    let mut w = Writer::new();
    if systems.is_empty() {
        return w.into_bytes();
    }
    w.chunk(0, |w| w.w_u32(PS_STATIC_VERSION));
    for (i, s) in systems.iter().enumerate() {
        w.chunk(i as u32 + 1, |w| {
            w.w_u16(s.game_type);
            w.w_stringz(&s.reference);
            w.w_matrix(&s.transform);
        });
    }
    w.into_bytes()
}

/// A group of fog volumes: an emitter and the volumes that occlude it.
#[derive(Clone, Debug, PartialEq)]
pub struct FogGroup {
    /// The particle profile of the emitter.
    pub profile: SmolStr,
    pub emitter: [[f32; 4]; 4],
    pub occluders: Vec<[[f32; 4]; 4]>,
}

/// The version `level.fog_vol` starts with.
pub const FOG_VOL_VERSION: u16 = 3;

/// Reads `level.fog_vol`: `u16 version, u32 groups`, then per group a profile line, the emitter's
/// matrix, `u32` occluders and their matrices.
pub fn read_fog_volumes(bytes: &[u8]) -> Result<Vec<FogGroup>> {
    let mut r = Reader::new(bytes);
    let version = r.r_u16()?;
    if version != FOG_VOL_VERSION {
        return Err(LevelError::Version { what: "level.fog_vol", found: u32::from(version) });
    }
    let groups = r.r_u32()?;
    let mut out = Vec::new();
    for _ in 0..groups {
        if r.eof() {
            return Err(LevelError::Invalid("level.fog_vol ends before its last group".into()));
        }
        let profile = SmolStr::new(r.r_line());
        let emitter = r.r_matrix()?;
        let count = r.r_u32()?;
        let n = crate::bytes::fits(r.elapsed(), u64::from(count), 64, "fog occluders")?;
        let mut occluders = Vec::with_capacity(n);
        for _ in 0..n {
            occluders.push(r.r_matrix()?);
        }
        out.push(FogGroup { profile, emitter, occluders });
    }
    Ok(out)
}

/// Writes `level.fog_vol`.
#[must_use]
pub fn write_fog_volumes(groups: &[FogGroup]) -> Vec<u8> {
    let mut w = Writer::new();
    w.w_u16(FOG_VOL_VERSION);
    w.w_u32(groups.len() as u32);
    for g in groups {
        w.w_line(&g.profile);
        w.w_matrix(&g.emitter);
        w.w_u32(g.occluders.len() as u32);
        for m in &g.occluders {
            w.w_matrix(m);
        }
    }
    w.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 4]; 4] =
        [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [2.0, 3.0, 4.0, 1.0]];

    fn game() -> LevelGame {
        LevelGame {
            ways: vec![
                WayObject {
                    name: "esc_patrol_walk".into(),
                    points: vec![
                        WayPoint { pos: Vec3::new(1.0, 2.0, 3.0), flags: 1, name: "wp00|a=patrol".into() },
                        WayPoint { pos: Vec3::new(4.0, 2.0, 3.0), flags: 0, name: "".into() },
                    ],
                    links: vec![
                        WayLink { from: 0, to: 1, probability: 1.0 },
                        WayLink { from: 1, to: 0, probability: 0.5 },
                    ],
                },
                WayObject { name: "empty".into(), ..WayObject::default() },
            ],
            rpoints: vec![RPoint {
                pos: Vec3::ONE,
                rot: Vec3::new(0.0, 1.57, 0.0),
                team: 1,
                kind: 2,
                game_type: 0xffff,
                profile: Some("mp_items".into()),
            }],
        }
    }

    #[test]
    fn level_game_round_trips() {
        let file = game();
        let bytes = file.write();
        let r = Reader::new(&bytes);
        assert_eq!(r.chunks().map(|c| c.0).collect::<Vec<_>>(), [0x1000, 0x2000]);
        // a way object: version, name, points, links
        let ways = r.sub(0x1000).unwrap().unwrap();
        let first = ways.sub(0).unwrap().unwrap();
        assert_eq!(first.chunks().map(|c| c.0).collect::<Vec<_>>(), [1, 5, 2, 3]);
        assert_eq!(first.sub(1).unwrap().unwrap().data(), [0x13, 0x00]);
        assert_eq!(first.sub(3).unwrap().unwrap().len(), 2 + 2 * 8);

        let back = LevelGame::read(&bytes).unwrap();
        assert_eq!(back, file);
        assert_eq!(back.write(), bytes);
    }

    #[test]
    fn an_old_respawn_point_has_no_profile() {
        let mut file = game();
        file.rpoints[0].profile = None;
        let bytes = file.write();
        let back = LevelGame::read(&bytes).unwrap();
        assert_eq!(back.rpoints[0].profile, None);
        assert_eq!(back.write(), bytes);
    }

    #[test]
    fn the_side_files_round_trip() {
        let mods = EnvMods {
            version: Some(SPAWNPOINT_VERSION),
            mods: vec![EnvModifier {
                pos: Vec3::X,
                radius: 10.0,
                power: 0.5,
                view_dist: 300.0,
                fog_color: Vec3::splat(0.5),
                fog_density: 0.1,
                ambient: Vec3::ZERO,
                sky_color: Vec3::ONE,
                hemi_color: Vec3::Y,
                flags: 0b101,
            }],
        };
        let bytes = mods.write();
        assert_eq!(bytes.len(), 12 + 8 + 78);
        assert_eq!(EnvMods::read(&bytes).unwrap(), mods);
        // a file that is only its version chunk, as the build writes for a level without any
        let empty = EnvMods { version: Some(SPAWNPOINT_VERSION), mods: vec![] };
        assert_eq!(empty.write(), [0, 0, 0, 0, 4, 0, 0, 0, 0x17, 0, 0, 0]);

        let sounds = vec![StaticSound {
            wave: "ambient\\wind".into(),
            pos: Vec3::Z,
            volume: 1.0,
            freq: 1.0,
            active_time: [0, 0],
            play_time: [1000, 2000],
            random_pause: [500, 1500],
        }];
        let bytes = write_static_sounds(&sounds);
        assert_eq!(read_static_sounds(&bytes).unwrap(), sounds);

        let particles = vec![StaticParticles {
            game_type: 0xffff,
            reference: "static\\fire".into(),
            transform: IDENTITY,
        }];
        let bytes = write_static_particles(&particles);
        assert_eq!(read_static_particles(&bytes).unwrap(), particles);
        assert!(write_static_particles(&[]).is_empty());

        let fog = vec![
            FogGroup { profile: "fog\\swamp".into(), emitter: IDENTITY, occluders: vec![IDENTITY, IDENTITY] },
            FogGroup { profile: "fog\\thin".into(), emitter: IDENTITY, occluders: vec![] },
        ];
        let bytes = write_fog_volumes(&fog);
        assert_eq!(&bytes[..6], [3, 0, 2, 0, 0, 0]);
        assert_eq!(&bytes[6..17], b"fog\\swamp\r\n");
        assert_eq!(read_fog_volumes(&bytes).unwrap(), fog);
    }

    #[test]
    fn hostile_input_is_an_error() {
        let bytes = game().write();
        // cut inside the points of the first way object
        assert!(LevelGame::read(&bytes[..60]).is_ok(), "a chunk that does not fit ends the walk");
        let mut wild = bytes.clone();
        // the point count of the first way: way chunk, object chunk, version chunk, name chunk
        let at = 8 + 8 + 10 + 8 + "esc_patrol_walk\0".len() + 8;
        wild[at..at + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(LevelGame::read(&wild).is_err());
        let mut version = bytes;
        version[8 + 8 + 8] = 0x12;
        assert!(matches!(LevelGame::read(&version), Err(LevelError::Version { found: 0x12, .. })));

        assert!(read_fog_volumes(&[3, 0, 0xff, 0xff, 0xff, 0xff]).is_err());
        assert!(read_fog_volumes(&[2, 0, 0, 0, 0, 0]).is_err());
        let mut fog =
            write_fog_volumes(&[FogGroup { profile: "p".into(), emitter: IDENTITY, occluders: vec![] }]);
        let n = fog.len();
        fog[n - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_fog_volumes(&fog).is_err());
    }
}
