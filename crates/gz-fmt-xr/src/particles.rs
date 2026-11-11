//! `particles.xr`: particle effects and groups. `PSLibrary.cpp` (`CPSLibrary::Load`),
//! `ParticleEffectDef.cpp` (`CPEDef::Load`), `ParticleGroup.cpp` (`CPGDef::Load`), and the
//! compiled action stream of `particle_actions_collection_io.cpp` /
//! `CParticleManager::LoadActions`.

use glam::{Vec2, Vec3};
use gz_chunk::Reader;
use smol_str::SmolStr;

use crate::{Encoding, Result, Text, invalid, numbered};

const PS_CHUNK_VERSION: u32 = 1;
const PS_CHUNK_SECONDGEN: u32 = 3;
const PS_CHUNK_THIRDGEN: u32 = 4;
/// `PS_VERSION`.
pub const PS_VERSION: u16 = 1;

const PED_VERSION: u16 = 1;
const PED_CHUNK_VERSION: u32 = 0x0001;
const PED_CHUNK_NAME: u32 = 0x0002;
const PED_CHUNK_EFFECTDATA: u32 = 0x0003;
const PED_CHUNK_ACTIONLIST: u32 = 0x0004;
const PED_CHUNK_FLAGS: u32 = 0x0005;
const PED_CHUNK_FRAME: u32 = 0x0006;
const PED_CHUNK_SPRITE: u32 = 0x0007;
const PED_CHUNK_TIMELIMIT: u32 = 0x0008;
const PED_CHUNK_COLLISION: u32 = 0x0021;
const PED_CHUNK_VEL_SCALE: u32 = 0x0022;
const PED_CHUNK_EDATA: u32 = 0x0024;
const PED_CHUNK_ALIGN_TO_PATH: u32 = 0x0025;

const PGD_VERSION: u16 = 3;
const PGD_CHUNK_VERSION: u32 = 0x0001;
const PGD_CHUNK_NAME: u32 = 0x0002;
const PGD_CHUNK_FLAGS: u32 = 0x0003;
const PGD_CHUNK_EFFECTS: u32 = 0x0004;
const PGD_CHUNK_TIME_LIMIT: u32 = 0x0005;

/// `CPEDef` flags.
pub mod effect_flags {
    pub const SPRITE: u32 = 1 << 0;
    pub const FRAMED: u32 = 1 << 10;
    pub const ANIMATED: u32 = 1 << 11;
    pub const RANDOM_FRAME: u32 = 1 << 12;
    pub const RANDOM_PLAYBACK: u32 = 1 << 13;
    pub const TIME_LIMIT: u32 = 1 << 14;
    pub const ALIGN_TO_PATH: u32 = 1 << 15;
    pub const COLLISION: u32 = 1 << 16;
    pub const COLLISION_DEL: u32 = 1 << 17;
    pub const VELOCITY_SCALE: u32 = 1 << 18;
    pub const COLLISION_DYN: u32 = 1 << 19;
    pub const WORLD_ALIGN: u32 = 1 << 20;
    pub const FACE_ALIGN: u32 = 1 << 21;
    pub const CULLING: u32 = 1 << 22;
}

/// `pDomain` (68 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Domain {
    /// `PDomainEnum`: point, line, triangle, plane, box, sphere, cylinder, cone, blob, disc,
    /// rectangle.
    pub kind: u32,
    pub p1: Vec3,
    pub p2: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub radius1: f32,
    pub radius2: f32,
    pub radius1_sqr: f32,
    pub radius2_sqr: f32,
}

/// `PActionEnum`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Avoid,
    Bounce,
    CopyVertexB,
    Damping,
    Explosion,
    Follow,
    Gravitate,
    Gravity,
    Jet,
    KillOld,
    MatchVelocity,
    Move,
    OrbitLine,
    OrbitPoint,
    RandomAccel,
    RandomDisplace,
    RandomVelocity,
    Restore,
    Sink,
    SinkVelocity,
    Source,
    SpeedLimit,
    TargetColor,
    TargetSize,
    TargetRotate,
    TargetRotateD,
    TargetVelocity,
    TargetVelocityD,
    Vortex,
    Turbulence,
    Scatter,
}

impl ActionKind {
    fn from_id(id: u32) -> Option<Self> {
        use ActionKind as A;
        Some(match id {
            0 => A::Avoid,
            1 => A::Bounce,
            3 => A::CopyVertexB,
            4 => A::Damping,
            5 => A::Explosion,
            6 => A::Follow,
            7 => A::Gravitate,
            8 => A::Gravity,
            9 => A::Jet,
            10 => A::KillOld,
            11 => A::MatchVelocity,
            12 => A::Move,
            13 => A::OrbitLine,
            14 => A::OrbitPoint,
            15 => A::RandomAccel,
            16 => A::RandomDisplace,
            17 => A::RandomVelocity,
            18 => A::Restore,
            19 => A::Sink,
            20 => A::SinkVelocity,
            21 => A::Source,
            22 => A::SpeedLimit,
            23 => A::TargetColor,
            24 => A::TargetSize,
            25 => A::TargetRotate,
            26 => A::TargetRotateD,
            27 => A::TargetVelocity,
            28 => A::TargetVelocityD,
            29 => A::Vortex,
            30 => A::Turbulence,
            31 => A::Scatter,
            _ => return None,
        })
    }
}

/// The values an action reads, by its kind.
// one value per action of a particle effect, read once: the size of the largest kind costs
// nothing worth a box per action
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum ActionParams {
    /// Avoid: domain, look ahead, magnitude, epsilon.
    Avoid {
        position: Domain,
        look_ahead: f32,
        magnitude: f32,
        epsilon: f32,
    },
    Bounce {
        position: Domain,
        one_minus_friction: f32,
        resilience: f32,
        cutoff_sqr: f32,
    },
    CopyVertexB {
        copy_pos: u32,
    },
    Damping {
        damping: Vec3,
        vlow_sqr: f32,
        vhigh_sqr: f32,
    },
    Explosion {
        center: Vec3,
        velocity: f32,
        magnitude: f32,
        stdev: f32,
        age: f32,
        epsilon: f32,
    },
    /// Follow, Gravitate, MatchVelocity.
    Field {
        magnitude: f32,
        epsilon: f32,
        max_radius: f32,
    },
    Gravity {
        direction: Vec3,
    },
    Jet {
        center: Vec3,
        acc: Domain,
        magnitude: f32,
        epsilon: f32,
        max_radius: f32,
    },
    KillOld {
        age_limit: f32,
        kill_less_than: u32,
    },
    Move,
    /// OrbitLine, Vortex.
    Axis {
        point: Vec3,
        axis: Vec3,
        magnitude: f32,
        epsilon: f32,
        max_radius: f32,
    },
    /// OrbitPoint, Scatter.
    Point {
        center: Vec3,
        magnitude: f32,
        epsilon: f32,
        max_radius: f32,
    },
    /// RandomAccel, RandomDisplace, RandomVelocity.
    Random {
        domain: Domain,
    },
    Restore {
        time_left: f32,
    },
    /// Sink, SinkVelocity.
    Sink {
        kill_inside: u32,
        domain: Domain,
    },
    Source {
        position: Domain,
        velocity: Domain,
        rot: Domain,
        size: Domain,
        color: Domain,
        alpha: f32,
        particle_rate: f32,
        age: f32,
        age_sigma: f32,
        parent_vel: Vec3,
        parent_motion: f32,
    },
    SpeedLimit {
        min_speed: f32,
        max_speed: f32,
    },
    TargetColor {
        color: Vec3,
        alpha: f32,
        scale: f32,
        /// 0 when not stored.
        time_from: f32,
        /// 1 when not stored.
        time_to: f32,
        /// How many of the two time fields the file holds: Shadow of Chernobyl's 20-byte form
        /// stores none, and Dead Air's engine reads what is there before the list ends.
        times_stored: u8,
    },
    TargetSize {
        size: Vec3,
        scale: Vec3,
    },
    /// TargetRotate, TargetRotateD, TargetVelocity, TargetVelocityD.
    Target {
        value: Vec3,
        scale: f32,
    },
    Turbulence {
        frequency: f32,
        octaves: i32,
        magnitude: f32,
        epsilon: f32,
        offset: Vec3,
    },
}

/// One compiled action.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub kind: ActionKind,
    pub flags: u32,
    pub params: ActionParams,
}

/// `CPEDef::SFrame`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectFrame {
    pub tex_size: Vec2,
    pub reserved: Vec2,
    pub dim_x: i32,
    pub frame_count: i32,
    pub speed: f32,
}

/// `CPEDef`.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleEffect {
    pub name: SmolStr,
    pub max_particles: u32,
    pub flags: u32,
    pub shader: SmolStr,
    pub texture: SmolStr,
    pub frame: Option<EffectFrame>,
    pub time_limit: Option<f32>,
    /// One minus friction, resilience, squared cutoff.
    pub collision: Option<[f32; 3]>,
    pub velocity_scale: Option<Vec3>,
    pub align_to_path_rotation: Option<Vec3>,
    pub actions: Vec<Action>,
    /// `PED_CHUNK_EDATA`: the editor's action list, kept as stored.
    pub editor_actions: Option<Vec<u8>>,
}

/// `CPGDef::SEffect`.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupEffect {
    pub effect: SmolStr,
    pub on_play_child: SmolStr,
    pub on_birth_child: SmolStr,
    pub on_dead_child: SmolStr,
    pub time0: f32,
    pub time1: f32,
    pub flags: u32,
}

/// `CPGDef`.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleGroup {
    pub name: SmolStr,
    pub flags: u32,
    /// As loaded: the stored limit, or the latest end of an effect when none is stored.
    pub time_limit: f32,
    pub effects: Vec<GroupEffect>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Particles {
    pub effects: Vec<ParticleEffect>,
    pub groups: Vec<ParticleGroup>,
    pub encoding: Encoding,
}

fn domain(r: &mut Reader<'_>) -> Result<Domain> {
    Ok(Domain {
        kind: r.r_u32()?,
        p1: r.r_vec3()?,
        p2: r.r_vec3()?,
        u: r.r_vec3()?,
        v: r.r_vec3()?,
        radius1: r.r_f32()?,
        radius2: r.r_f32()?,
        radius1_sqr: r.r_f32()?,
        radius2_sqr: r.r_f32()?,
    })
}

/// The two layouts of `PATargetColor`: Shadow of Chernobyl's 20 bytes, or 28 with the time
/// window after them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ColorForm {
    Short,
    Long,
}

fn action(r: &mut Reader<'_>, color_form: ColorForm) -> Result<Action> {
    let id = r.r_u32()?;
    let kind = ActionKind::from_id(id)
        .ok_or_else(|| invalid(format!("particle action type {id} is not one the game has")))?;
    // `ParticleAction::Load`: the flags and the type again
    let flags = r.r_u32()?;
    r.r_u32()?;
    use ActionKind as A;
    use ActionParams as P;
    let f = |r: &mut Reader<'_>| r.r_f32();
    let params = match kind {
        A::Avoid => P::Avoid { position: domain(r)?, look_ahead: f(r)?, magnitude: f(r)?, epsilon: f(r)? },
        A::Bounce => P::Bounce {
            position: domain(r)?,
            one_minus_friction: f(r)?,
            resilience: f(r)?,
            cutoff_sqr: f(r)?,
        },
        A::CopyVertexB => P::CopyVertexB { copy_pos: r.r_u32()? },
        A::Damping => P::Damping { damping: r.r_vec3()?, vlow_sqr: f(r)?, vhigh_sqr: f(r)? },
        A::Explosion => P::Explosion {
            center: r.r_vec3()?,
            velocity: f(r)?,
            magnitude: f(r)?,
            stdev: f(r)?,
            age: f(r)?,
            epsilon: f(r)?,
        },
        A::Follow | A::Gravitate | A::MatchVelocity => {
            P::Field { magnitude: f(r)?, epsilon: f(r)?, max_radius: f(r)? }
        }
        A::Gravity => P::Gravity { direction: r.r_vec3()? },
        A::Jet => P::Jet {
            center: r.r_vec3()?,
            acc: domain(r)?,
            magnitude: f(r)?,
            epsilon: f(r)?,
            max_radius: f(r)?,
        },
        A::KillOld => P::KillOld { age_limit: f(r)?, kill_less_than: r.r_u32()? },
        A::Move => P::Move,
        A::OrbitLine | A::Vortex => P::Axis {
            point: r.r_vec3()?,
            axis: r.r_vec3()?,
            magnitude: f(r)?,
            epsilon: f(r)?,
            max_radius: f(r)?,
        },
        A::OrbitPoint | A::Scatter => {
            P::Point { center: r.r_vec3()?, magnitude: f(r)?, epsilon: f(r)?, max_radius: f(r)? }
        }
        A::RandomAccel | A::RandomDisplace | A::RandomVelocity => P::Random { domain: domain(r)? },
        A::Restore => P::Restore { time_left: f(r)? },
        A::Sink | A::SinkVelocity => P::Sink { kill_inside: r.r_u32()?, domain: domain(r)? },
        A::Source => P::Source {
            position: domain(r)?,
            velocity: domain(r)?,
            rot: domain(r)?,
            size: domain(r)?,
            color: domain(r)?,
            alpha: f(r)?,
            particle_rate: f(r)?,
            age: f(r)?,
            age_sigma: f(r)?,
            parent_vel: r.r_vec3()?,
            parent_motion: f(r)?,
        },
        A::SpeedLimit => P::SpeedLimit { min_speed: f(r)?, max_speed: f(r)? },
        A::TargetColor => {
            let (color, alpha, scale) = (r.r_vec3()?, f(r)?, f(r)?);
            let (time_from, time_to, times_stored) = match color_form {
                // `PATargetColor`'s constructor: the whole life
                ColorForm::Short => (0.0, 1.0, 0),
                ColorForm::Long => (f(r)?, f(r)?, 2),
            };
            P::TargetColor { color, alpha, scale, time_from, time_to, times_stored }
        }
        A::TargetSize => P::TargetSize { size: r.r_vec3()?, scale: r.r_vec3()? },
        A::TargetRotate | A::TargetRotateD | A::TargetVelocity | A::TargetVelocityD => {
            P::Target { value: r.r_vec3()?, scale: f(r)? }
        }
        A::Turbulence => P::Turbulence {
            frequency: f(r)?,
            octaves: r.r_i32()?,
            magnitude: f(r)?,
            epsilon: f(r)?,
            offset: r.r_vec3()?,
        },
    };
    Ok(Action { kind, flags, params })
}

/// The type of the action at the reader's place, without moving it.
fn peek_kind(r: &Reader<'_>) -> Option<ActionKind> {
    let mut p = r.clone();
    p.r_u32().ok().and_then(ActionKind::from_id)
}

/// How many action reads one list may cost before the search for an exact reading gives up: a
/// list of honest data needs one per action and a few more.
const SEARCH_BUDGET: usize = 1 << 14;

/// Reads `n` actions. A `TargetColor` is read long first, short when what follows does not read
/// (a depth-first search over those choices, without recursion). `exact`: the list must end
/// where its bytes end. `Err` holds the first failure of the long-first reading.
fn read_list(
    start: &Reader<'_>,
    n: usize,
    exact: bool,
) -> std::result::Result<Vec<Action>, Option<crate::XrError>> {
    let mut r = start.clone();
    let mut out: Vec<Action> = Vec::with_capacity(n);
    // where a short reading of a colour action is still to be tried
    let mut choices: Vec<(Reader<'_>, usize)> = Vec::new();
    let mut first_error = None;
    let mut work = 0usize;
    loop {
        let stepped = if out.len() == n {
            if !exact || r.elapsed() == 0 {
                return Ok(out);
            }
            false
        } else {
            work += 1;
            if work > SEARCH_BUDGET {
                return Err(first_error);
            }
            if peek_kind(&r) == Some(ActionKind::TargetColor) {
                choices.push((r.clone(), out.len()));
            }
            match action(&mut r, ColorForm::Long) {
                Ok(a) => {
                    out.push(a);
                    true
                }
                Err(e) => {
                    first_error.get_or_insert(e);
                    false
                }
            }
        };
        if stepped {
            continue;
        }
        // back to the newest colour action, read short this time
        loop {
            let Some((at, len)) = choices.pop() else { return Err(first_error) };
            r = at;
            out.truncate(len);
            if let Ok(a) = action(&mut r, ColorForm::Short) {
                out.push(a);
                break;
            }
        }
    }
}

/// `LoadActions`: a count and the actions. Dead Air's library keeps Shadow of Chernobyl's
/// 20-byte `TargetColor` in 50 effects, also before other actions (the game's own reader takes
/// the short form only at the list's end): each colour action takes the form that makes the
/// whole list read to its end; a list that reads in no form exactly is read as far as it goes,
/// as the engine, which never checks the end, reads it.
fn actions(body: &[u8]) -> Result<Vec<Action>> {
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let mut r = Reader::new(body);
    let n = r.r_u32()? as usize;
    // a type, the flags and the type again at least
    if n.saturating_mul(12) > r.elapsed() {
        return Err(invalid(format!("a particle effect states {n} actions and cannot hold them")));
    }
    read_list(&r, n, true)
        .or_else(|_| read_list(&r, n, false))
        .map_err(|e| e.unwrap_or_else(|| invalid("the actions do not read in any form")))
}

fn need<'a>(r: &Reader<'a>, id: u32, what: &str) -> Result<Reader<'a>> {
    r.sub(id)?.ok_or_else(|| invalid(format!("a particle definition has no {what}")))
}

fn effect(body: &[u8], text: &mut Text) -> Result<Option<ParticleEffect>> {
    let r = Reader::new(body);
    if need(&r, PED_CHUNK_VERSION, "version")?.r_u16()? != PED_VERSION {
        return Ok(None);
    }
    let name = text.r_str(&mut need(&r, PED_CHUNK_NAME, "name")?)?;
    let max_particles = need(&r, PED_CHUNK_EFFECTDATA, "effect data")?.r_u32()?;
    let list = need(&r, PED_CHUNK_ACTIONLIST, "action list")?;
    let actions = actions(list.data()).map_err(|e| invalid(format!("particle effect '{name}': {e}")))?;
    let flags = match r.sub(PED_CHUNK_FLAGS)? {
        Some(mut f) => f.r_u32()?,
        None => 0,
    };
    let mut e = ParticleEffect {
        name,
        max_particles,
        flags,
        shader: SmolStr::default(),
        texture: SmolStr::default(),
        frame: None,
        time_limit: None,
        collision: None,
        velocity_scale: None,
        align_to_path_rotation: None,
        actions,
        editor_actions: r.sub(PED_CHUNK_EDATA)?.map(|c| c.data().to_vec()),
    };
    use effect_flags as F;
    if flags & F::SPRITE != 0 {
        let mut s = need(&r, PED_CHUNK_SPRITE, "sprite")?;
        e.shader = text.r_str(&mut s)?;
        e.texture = text.r_str(&mut s)?;
    }
    if flags & F::FRAMED != 0 {
        let mut fr = need(&r, PED_CHUNK_FRAME, "frame")?;
        e.frame = Some(EffectFrame {
            tex_size: fr.r_vec2()?,
            reserved: fr.r_vec2()?,
            dim_x: fr.r_i32()?,
            frame_count: fr.r_i32()?,
            speed: fr.r_f32()?,
        });
    }
    if flags & F::TIME_LIMIT != 0 {
        e.time_limit = Some(need(&r, PED_CHUNK_TIMELIMIT, "time limit")?.r_f32()?);
    }
    if flags & F::COLLISION != 0 {
        let mut c = need(&r, PED_CHUNK_COLLISION, "collision")?;
        e.collision = Some([c.r_f32()?, c.r_f32()?, c.r_f32()?]);
    }
    if flags & F::VELOCITY_SCALE != 0 {
        e.velocity_scale = Some(need(&r, PED_CHUNK_VEL_SCALE, "velocity scale")?.r_vec3()?);
    }
    if flags & F::ALIGN_TO_PATH != 0
        && let Some(mut a) = r.sub(PED_CHUNK_ALIGN_TO_PATH)?
    {
        e.align_to_path_rotation = Some(a.r_vec3()?);
    }
    Ok(Some(e))
}

fn group(body: &[u8], text: &mut Text) -> Result<Option<ParticleGroup>> {
    let r = Reader::new(body);
    if need(&r, PGD_CHUNK_VERSION, "version")?.r_u16()? != PGD_VERSION {
        return Ok(None);
    }
    let name = text.r_str(&mut need(&r, PGD_CHUNK_NAME, "name")?)?;
    let flags = match r.sub(PGD_CHUNK_FLAGS)? {
        Some(mut f) => f.r_u32()?,
        None => 0,
    };
    let mut time_limit = match r.sub(PGD_CHUNK_TIME_LIMIT)? {
        Some(mut t) => t.r_f32()?,
        None => 0.0,
    };
    let stored = time_limit > 0.0;
    let mut effects = Vec::new();
    if let Some(mut er) = r.sub(PGD_CHUNK_EFFECTS)? {
        let n = er.r_u32()? as usize;
        if n.saturating_mul(16) > er.elapsed() {
            return Err(invalid(format!("particle group '{name}' states {n} effects and cannot hold them")));
        }
        for _ in 0..n {
            let ge = GroupEffect {
                effect: text.r_str(&mut er)?,
                on_play_child: text.r_str(&mut er)?,
                on_birth_child: text.r_str(&mut er)?,
                on_dead_child: text.r_str(&mut er)?,
                time0: er.r_f32()?,
                time1: er.r_f32()?,
                flags: er.r_u32()?,
            };
            if !stored {
                time_limit = time_limit.max(ge.time1);
            }
            effects.push(ge);
        }
    }
    Ok(Some(ParticleGroup { name, flags, time_limit, effects }))
}

impl Particles {
    /// `CPSLibrary::Load`: a definition of another version is skipped, as the game skips it.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let version = need(&r, PS_CHUNK_VERSION, "library version")?.r_u16()?;
        if version != PS_VERSION {
            return Err(invalid(format!("particles.xr has version {version}; the game reads version 1")));
        }
        let mut text = Text::default();
        let mut out = Self::default();
        if let Some(c) = r.open_chunk(PS_CHUNK_SECONDGEN)? {
            for e in numbered(&c.reader())? {
                if let Some(e) = effect(e.bytes(), &mut text)? {
                    out.effects.push(e);
                }
            }
        }
        if let Some(c) = r.open_chunk(PS_CHUNK_THIRDGEN)? {
            for g in numbered(&c.reader())? {
                if let Some(g) = group(g.bytes(), &mut text)? {
                    out.groups.push(g);
                }
            }
        }
        out.encoding = text.encoding();
        Ok(out)
    }

    #[must_use]
    pub fn effect(&self, name: &str) -> Option<&ParticleEffect> {
        self.effects.iter().find(|e| e.name == name)
    }

    #[must_use]
    pub fn group(&self, name: &str) -> Option<&ParticleGroup> {
        self.groups.iter().find(|g| g.name == name)
    }
}

#[cfg(test)]
mod tests {
    use gz_chunk::Writer;

    use super::*;

    fn w_domain(w: &mut Writer, kind: u32) {
        w.w_u32(kind);
        for _ in 0..4 {
            w.w_vec3(Vec3::ONE);
        }
        for _ in 0..4 {
            w.w_f32(2.0);
        }
    }

    #[test]
    fn effects_groups_and_actions_are_read() {
        let mut w = Writer::new();
        w.chunk(PS_CHUNK_VERSION, |w| w.w_u16(1));
        w.chunk(PS_CHUNK_SECONDGEN, |w| {
            w.chunk(0, |w| {
                w.chunk(PED_CHUNK_VERSION, |w| w.w_u16(1));
                w.chunk(PED_CHUNK_NAME, |w| w.w_stringz("smoke\\puff"));
                w.chunk(PED_CHUNK_EFFECTDATA, |w| w.w_u32(64));
                w.chunk(PED_CHUNK_ACTIONLIST, |w| {
                    w.w_u32(2);
                    // gravity
                    w.w_u32(8);
                    w.w_u32(1);
                    w.w_u32(8);
                    w.w_vec3(Vec3::new(0.0, -9.8, 0.0));
                    // random velocity
                    w.w_u32(17);
                    w.w_u32(0);
                    w.w_u32(17);
                    w_domain(w, 5);
                });
                w.chunk(PED_CHUNK_FLAGS, |w| w.w_u32(effect_flags::SPRITE | effect_flags::TIME_LIMIT));
                w.chunk(PED_CHUNK_SPRITE, |w| {
                    w.w_stringz("particles\\add");
                    w.w_stringz("pfx\\smoke");
                });
                w.chunk(PED_CHUNK_TIMELIMIT, |w| w.w_f32(2.5));
            });
        });
        w.chunk(PS_CHUNK_THIRDGEN, |w| {
            w.chunk(0, |w| {
                w.chunk(PGD_CHUNK_VERSION, |w| w.w_u16(3));
                w.chunk(PGD_CHUNK_NAME, |w| w.w_stringz("explosions\\big"));
                w.chunk(PGD_CHUNK_FLAGS, |w| w.w_u32(0));
                w.chunk(PGD_CHUNK_EFFECTS, |w| {
                    w.w_u32(1);
                    for s in ["smoke\\puff", "", "", ""] {
                        w.w_stringz(s);
                    }
                    w.w_f32(0.0);
                    w.w_f32(3.0);
                    w.w_u32(4);
                });
            });
        });
        let p = Particles::read(&w.into_bytes()).unwrap();
        let e = p.effect("smoke\\puff").unwrap();
        assert_eq!((e.max_particles, e.time_limit, e.texture.as_str()), (64, Some(2.5), "pfx\\smoke"));
        assert_eq!(e.actions[0].params, ActionParams::Gravity { direction: Vec3::new(0.0, -9.8, 0.0) });
        assert!(
            matches!(&e.actions[1].params, ActionParams::Random { domain } if domain.kind == 5 && domain.radius2_sqr == 2.0)
        );
        assert_eq!(p.group("explosions\\big").map(|g| g.time_limit), Some(3.0));
    }

    /// A list of a `TargetColor` (with or without its time window) and, optionally, a `Move`.
    fn colour_list(long: bool, then_move: bool) -> Vec<u8> {
        let mut w = Writer::new();
        w.w_u32(1 + u32::from(then_move));
        for v in [23, 0, 23] {
            w.w_u32(v);
        }
        w.w_vec3(Vec3::new(0.5, 0.25, 1.0));
        w.w_f32(0.75);
        w.w_f32(2.0);
        if long {
            w.w_f32(0.25);
            w.w_f32(0.5);
        }
        if then_move {
            for v in [12, 0, 12] {
                w.w_u32(v);
            }
        }
        w.into_bytes()
    }

    #[test]
    fn both_forms_of_the_target_colour_read() {
        for then_move in [false, true] {
            let short = actions(&colour_list(false, then_move)).unwrap();
            assert_eq!(short.len(), 1 + usize::from(then_move));
            assert!(matches!(
                short[0].params,
                ActionParams::TargetColor { time_from: 0.0, time_to: 1.0, times_stored: 0, scale: 2.0, .. }
            ));
            let long = actions(&colour_list(true, then_move)).unwrap();
            assert!(matches!(
                long[0].params,
                ActionParams::TargetColor { time_from: 0.25, time_to: 0.5, times_stored: 2, .. }
            ));
            if then_move {
                assert_eq!((short[1].kind, long[1].kind), (ActionKind::Move, ActionKind::Move));
            }
        }
    }

    #[test]
    fn a_long_list_of_colour_actions_that_never_fits_ends() {
        // every colour action could be either form and the last is cut: the search is bounded
        let mut w = Writer::new();
        w.w_u32(4000);
        for _ in 0..4000 {
            for v in [23, 0, 23] {
                w.w_u32(v);
            }
            for _ in 0..5 {
                w.w_f32(1.0);
            }
        }
        let mut bytes = w.into_bytes();
        bytes.truncate(bytes.len() - 6);
        assert!(actions(&bytes).is_err());
    }

    #[test]
    fn an_unknown_action_is_an_error() {
        let mut w = Writer::new();
        w.w_u32(1);
        w.w_u32(2);
        w.w_u32(0);
        w.w_u32(2);
        assert!(actions(&w.into_bytes()).is_err());
        assert!(actions(&[0xff, 0xff, 0xff, 0x0f]).is_err());
    }
}
