//! The definitions of a `particles.xr`, the library that finds them
//!  and a playing [`System`] - an effect or a group - with its own random sources.

use std::sync::Arc;

use glam::{Mat4, Vec2, Vec3};
use smol_str::SmolStr;

use crate::actions::Ctx;
use crate::effect::{Effect, Particle, SpriteBatch};
use crate::group::Group;
use crate::noise::Noise;
use crate::rand::Rngs;

/// The effect definition flags.
pub mod flags {
    pub use gz_fmt_xr::effect_flags::*;
    pub const CULL_CCW: u32 = 1 << 23;
}

/// The group's effect flags.
pub mod group_flags {
    pub const DEFERRED_STOP: u32 = 1 << 0;
    pub const ON_PLAY_CHILD: u32 = 1 << 1;
    pub const ENABLED: u32 = 1 << 2;
    pub const ON_PLAY_CHILD_REWIND: u32 = 1 << 4;
    pub const ON_BIRTH_CHILD: u32 = 1 << 5;
    pub const ON_DEAD_CHILD: u32 = 1 << 6;
}

/// The atlas a framed effect plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// One frame's size in texture coordinates.
    pub tex_size: Vec2,
    pub dim_x: i32,
    pub frame_count: i32,
    /// Frames a second.
    pub speed: f32,
}

impl Default for Frame {
    /// The default values.
    fn default() -> Self {
        Self { tex_size: Vec2::new(32.0 / 256.0, 64.0 / 128.0), dim_x: 8, frame_count: 16, speed: 24.0 }
    }
}

impl Frame {
    /// The left-top and right-bottom texture coordinates of a frame.
    #[must_use]
    pub fn tc(&self, frame: i32) -> (Vec2, Vec2) {
        let (col, row) = if self.dim_x == 0 { (0, 0) } else { (frame % self.dim_x, frame / self.dim_x) };
        let lt = Vec2::new(col as f32 * self.tex_size.x, row as f32 * self.tex_size.y);
        (lt, lt + self.tex_size)
    }
}

/// An effect as loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectDef {
    pub name: SmolStr,
    pub flags: u32,
    pub max_particles: u32,
    pub shader: SmolStr,
    pub texture: SmolStr,
    pub frame: Frame,
    pub time_limit: f32,
    /// One minus friction, resilience, squared cutoff (`m_fCollide*`).
    pub collide: [f32; 3],
    pub velocity_scale: Vec3,
    /// `m_APDefaultRotation`.
    pub ap_default_rotation: Vec3,
    pub actions: Vec<gz_fmt_xr::Action>,
}

impl From<&gz_fmt_xr::ParticleEffect> for EffectDef {
    /// Loads an effect definition, with the constructor's defaults for what the file does not hold.
    fn from(e: &gz_fmt_xr::ParticleEffect) -> Self {
        Self {
            name: e.name.clone(),
            flags: e.flags,
            max_particles: e.max_particles,
            shader: e.shader.clone(),
            texture: e.texture.clone(),
            frame: e.frame.map_or_else(Frame::default, |f| Frame {
                tex_size: f.tex_size,
                dim_x: f.dim_x,
                frame_count: f.frame_count,
                speed: f.speed,
            }),
            time_limit: e.time_limit.unwrap_or(0.0),
            collide: e.collision.unwrap_or([1.0, 0.0, 0.0]),
            velocity_scale: e.velocity_scale.unwrap_or(Vec3::ZERO),
            ap_default_rotation: e.align_to_path_rotation.unwrap_or(Vec3::new(
                -std::f32::consts::FRAC_PI_2,
                0.0,
                0.0,
            )),
            actions: e.actions.clone(),
        }
    }
}

/// A group's effect.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupEffect {
    pub effect: SmolStr,
    pub on_play_child: SmolStr,
    pub on_birth_child: SmolStr,
    pub on_dead_child: SmolStr,
    pub time0: f32,
    pub time1: f32,
    /// [`group_flags`].
    pub flags: u32,
}

/// A group as loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupDef {
    pub name: SmolStr,
    pub flags: u32,
    pub time_limit: f32,
    pub effects: Vec<GroupEffect>,
}

impl From<&gz_fmt_xr::ParticleGroup> for GroupDef {
    fn from(g: &gz_fmt_xr::ParticleGroup) -> Self {
        Self {
            name: g.name.clone(),
            flags: g.flags,
            time_limit: g.time_limit,
            effects: g
                .effects
                .iter()
                .map(|e| GroupEffect {
                    effect: e.effect.clone(),
                    on_play_child: e.on_play_child.clone(),
                    on_birth_child: e.on_birth_child.clone(),
                    on_dead_child: e.on_dead_child.clone(),
                    time0: e.time0,
                    time1: e.time1,
                    flags: e.flags,
                })
                .collect(),
        }
    }
}

/// Where a particle's move meets a surface: the nearest hit along `dir` (a unit vector) within
/// `dist`, its point and the surface's normal. The editor's `Tools->RayPick`.
pub trait Collider: Sync {
    fn ray_pick(&self, start: Vec3, dir: Vec3, dist: f32) -> Option<(Vec3, Vec3)>;
}

/// The effects and groups of one `particles.xr`, sorted by name.
#[derive(Debug)]
pub struct Library {
    effects: Vec<Arc<EffectDef>>,
    groups: Vec<Arc<GroupDef>>,
    pub(crate) noise: Noise,
}

impl Library {
    /// Loads the library over a read file: sorted the way the engine sorts, the
    /// first of two equal names kept in front.
    #[must_use]
    pub fn new(p: &gz_fmt_xr::Particles) -> Self {
        let mut effects: Vec<Arc<EffectDef>> =
            p.effects.iter().map(|e| Arc::new(EffectDef::from(e))).collect();
        let mut groups: Vec<Arc<GroupDef>> = p.groups.iter().map(|g| Arc::new(GroupDef::from(g))).collect();
        effects.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        groups.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        Self { effects, groups, noise: Noise::new() }
    }

    /// Finds an effect definition.
    #[must_use]
    pub fn effect(&self, name: &str) -> Option<&Arc<EffectDef>> {
        let i = self.effects.partition_point(|e| e.name.as_bytes() < name.as_bytes());
        self.effects.get(i).filter(|e| e.name == name)
    }

    /// Finds a group definition.
    #[must_use]
    pub fn group(&self, name: &str) -> Option<&Arc<GroupDef>> {
        let i = self.groups.partition_point(|g| g.name.as_bytes() < name.as_bytes());
        self.groups.get(i).filter(|g| g.name == name)
    }

    #[must_use]
    pub fn effects(&self) -> &[Arc<EffectDef>] {
        &self.effects
    }

    #[must_use]
    pub fn groups(&self) -> &[Arc<GroupDef>] {
        &self.groups
    }

    /// `model_CreateParticles`: an effect of that name, else a group; not playing yet.
    #[must_use]
    pub fn create(self: &Arc<Self>, name: &str) -> Option<System> {
        let kind = if let Some(e) = self.effect(name) {
            Kind::Effect(Box::new(Effect::new(Some(e.clone()))))
        } else {
            Kind::Group(Box::new(Group::new(self.group(name)?.clone(), self)))
        };
        Some(System { lib: self.clone(), rng: Rngs::default(), kind, name: SmolStr::new(name) })
    }
}

#[derive(Debug)]
enum Kind {
    Effect(Box<Effect>),
    Group(Box<Group>),
}

/// A particle system of the scene: an effect or a group, with its own random sources.
#[derive(Debug)]
pub struct System {
    lib: Arc<Library>,
    rng: Rngs,
    kind: Kind,
    name: SmolStr,
}

impl System {
    /// Both random sources start from `seed` (1 is what the engine starts from).
    #[must_use]
    pub fn with_seed(mut self, seed: i32) -> Self {
        self.rng = Rngs::seeded(seed);
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn is_group(&self) -> bool {
        matches!(self.kind, Kind::Group(_))
    }

    #[must_use]
    pub fn is_playing(&self) -> bool {
        match &self.kind {
            Kind::Effect(e) => e.is_playing(),
            Kind::Group(g) => g.is_playing(),
        }
    }

    pub fn play(&mut self) {
        match &mut self.kind {
            Kind::Effect(e) => e.play(),
            Kind::Group(g) => g.play(),
        }
    }

    /// `Stop`: `deferred` lets the particles live out their lives (what Stop in the editor does).
    pub fn stop(&mut self, deferred: bool) {
        match &mut self.kind {
            Kind::Effect(e) => e.stop(deferred),
            Kind::Group(g) => g.stop(deferred),
        }
    }

    /// The emitter's transform and velocity; `xform` draws the particles in the
    /// emitter's space instead of carrying the actions into the world (the editor passes false).
    pub fn update_parent(&mut self, m: &Mat4, vel: Vec3, xform: bool) {
        match &mut self.kind {
            Kind::Effect(e) => e.update_parent(m, vel, xform),
            Kind::Group(g) => g.update_parent(m, vel, xform),
        }
    }

    /// `dt_ms` milliseconds of play.
    pub fn on_frame(&mut self, dt_ms: u32, collider: Option<&dyn Collider>) {
        let mut cx = Ctx { rng: &mut self.rng, noise: &self.lib.noise };
        match &mut self.kind {
            Kind::Effect(e) => e.on_frame(dt_ms, &mut cx, collider),
            Kind::Group(g) => g.on_frame(dt_ms, &mut cx, &self.lib, collider),
        }
    }

    /// What to draw: one batch per effect with particles (a group's items and children too).
    pub fn sprites(&self, out: &mut Vec<SpriteBatch>) {
        match &self.kind {
            Kind::Effect(e) => out.extend(e.sprites()),
            Kind::Group(g) => g.sprites(out),
        }
    }

    /// Every live particle, as plain data.
    #[must_use]
    pub fn particles(&self) -> Vec<Particle> {
        match &self.kind {
            Kind::Effect(e) => e.store.list.clone(),
            Kind::Group(g) => g.particles(),
        }
    }

    #[must_use]
    pub fn particle_count(&self) -> usize {
        match &self.kind {
            Kind::Effect(e) => e.store.list.len(),
            Kind::Group(g) => g.particle_count(),
        }
    }

    /// `vis.box`: where the particles are, once a frame ran.
    #[must_use]
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        match &self.kind {
            Kind::Effect(e) => e.bounds,
            Kind::Group(g) => g.bounds,
        }
    }

    /// What the engine logs while it plays (a looped effect a group cannot spawn).
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        match &self.kind {
            Kind::Effect(_) => &[],
            Kind::Group(g) => &g.warnings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_fmt_xr::{Action, ActionKind, ActionParams, Domain as FDomain};

    fn point(p: Vec3) -> FDomain {
        FDomain { kind: 0, p1: p, ..FDomain::default() }
    }

    fn source(rate: f32) -> Action {
        Action {
            kind: ActionKind::Source,
            flags: 0,
            params: ActionParams::Source {
                position: point(Vec3::ZERO),
                velocity: point(Vec3::new(0.0, 1.0, 0.0)),
                rot: point(Vec3::ZERO),
                size: point(Vec3::splat(0.2)),
                color: point(Vec3::ONE),
                alpha: 1.0,
                particle_rate: rate,
                age: 0.0,
                age_sigma: 0.0,
                parent_vel: Vec3::ZERO,
                parent_motion: 0.0,
            },
        }
    }

    fn effect(name: &str, flags: u32, time_limit: Option<f32>) -> gz_fmt_xr::ParticleEffect {
        gz_fmt_xr::ParticleEffect {
            name: name.into(),
            max_particles: 100,
            flags,
            shader: "particles\\add".into(),
            texture: "pfx\\pfx_spark".into(),
            frame: None,
            time_limit,
            collision: None,
            velocity_scale: None,
            align_to_path_rotation: None,
            actions: vec![
                source(30.3),
                Action { kind: ActionKind::Move, flags: 0, params: ActionParams::Move },
                Action {
                    kind: ActionKind::KillOld,
                    flags: 0,
                    params: ActionParams::KillOld { age_limit: 0.5, kill_less_than: 0 },
                },
            ],
            editor_actions: None,
        }
    }

    fn library() -> Arc<Library> {
        let p = gz_fmt_xr::Particles {
            effects: vec![
                effect("b\\spark", flags::SPRITE, None),
                effect("a\\puff", flags::SPRITE | flags::TIME_LIMIT, Some(0.2)),
            ],
            groups: vec![gz_fmt_xr::ParticleGroup {
                name: "g\\boom".into(),
                flags: 0,
                time_limit: 1.0,
                effects: vec![gz_fmt_xr::GroupEffect {
                    effect: "b\\spark".into(),
                    on_play_child: "".into(),
                    on_birth_child: "a\\puff".into(),
                    on_dead_child: "".into(),
                    time0: 0.0,
                    time1: 0.5,
                    flags: group_flags::ENABLED | group_flags::ON_BIRTH_CHILD | group_flags::DEFERRED_STOP,
                }],
            }],
            encoding: gz_fmt_xr::Encoding::default(),
        };
        Arc::new(Library::new(&p))
    }

    #[test]
    fn the_library_is_sorted_and_creates_effects_first() {
        let lib = library();
        assert_eq!(lib.effects()[0].name, "a\\puff");
        assert!(lib.effect("b\\spark").is_some() && lib.effect("B\\spark").is_none());
        assert!(!lib.create("b\\spark").is_some_and(|s| s.is_group()));
        assert!(lib.create("g\\boom").is_some_and(|s| s.is_group()));
        assert!(lib.create("nothing").is_none());
    }

    #[test]
    fn an_effect_steps_every_33_ms() {
        let lib = library();
        let mut s = lib.create("b\\spark").unwrap();
        s.update_parent(&Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0)), Vec3::ZERO, false);
        // not playing: no particles, the box at the emitter
        s.on_frame(100, None);
        assert_eq!(s.particle_count(), 0);
        s.play();
        // 20 ms is no step yet
        s.on_frame(20, None);
        assert_eq!(s.particle_count(), 0);
        // 20 more: one step of 0.033 s: 30.3 * 0.033 = 0.9999 -> 0, dithered to 1 by the first
        // draw (41 / 32767 < 0.9999)
        s.on_frame(20, None);
        assert_eq!(s.particle_count(), 1);
        let p = s.particles()[0];
        // emitted at the moved source, then moved one step up
        assert!((p.pos - Vec3::new(5.0, 0.033, 0.0)).abs().max_element() < 1e-6);
        assert!((p.age - 0.033).abs() < 1e-7);
        // a long frame runs three steps at most
        s.on_frame(1000, None);
        assert!(s.particle_count() <= 4);
        let mut out = Vec::new();
        s.sprites(&mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].sprites.len(), s.particle_count());
        assert_eq!(out[0].sprites[0].half, Vec2::splat(0.1));
        // a deferred stop lets them die (KillOld at 0.5 s), then the effect stops
        s.stop(true);
        for _ in 0..30 {
            s.on_frame(33, None);
        }
        assert!(!s.is_playing());
        assert_eq!(s.particle_count(), 0);
    }

    #[test]
    fn seeds_pin_the_draws() {
        let lib = library();
        let run = |seed| {
            let mut s = lib.create("b\\spark").unwrap().with_seed(seed);
            s.play();
            for _ in 0..10 {
                s.on_frame(33, None);
            }
            s.particles()
        };
        assert_eq!(run(7), run(7));
    }

    #[test]
    fn a_time_limit_stops_the_effect() {
        let lib = library();
        let mut s = lib.create("a\\puff").unwrap();
        s.play();
        // 0.2 s is six steps; the seventh stops it (deferred)
        for _ in 0..7 {
            s.on_frame(33, None);
        }
        let n = s.particle_count();
        assert!(n > 0);
        for _ in 0..30 {
            s.on_frame(33, None);
        }
        assert!(!s.is_playing());
    }

    #[test]
    fn a_group_plays_its_effects_and_spawns_children() {
        let lib = library();
        let mut s = lib.create("g\\boom").unwrap();
        s.play();
        s.on_frame(33, None);
        // the item started at time0 = 0 and stepped once
        assert!(s.particle_count() >= 1);
        for _ in 0..10 {
            s.on_frame(33, None);
        }
        let mut out = Vec::new();
        s.sprites(&mut out);
        // the item's sprites and its children's (a\puff on every birth)
        assert!(out.len() >= 2, "{}", out.len());
        // past the group's time limit it stops, deferred, and ends
        for _ in 0..200 {
            s.on_frame(33, None);
        }
        assert!(!s.is_playing());
    }

    struct Floor;
    impl Collider for Floor {
        fn ray_pick(&self, start: Vec3, dir: Vec3, dist: f32) -> Option<(Vec3, Vec3)> {
            // the plane y = 0
            if dir.y >= 0.0 || start.y < 0.0 {
                return None;
            }
            let t = -start.y / dir.y;
            (t <= dist).then(|| (start + dir * t, Vec3::Y))
        }
    }

    #[test]
    fn collision_bounces_off_the_collider() {
        let mut e = effect("c\\drop", flags::SPRITE | flags::COLLISION, None);
        e.collision = Some([1.0, 0.5, 0.0]);
        e.actions[0] = {
            let mut a = source(1000.0);
            if let ActionParams::Source { velocity, position, .. } = &mut a.params {
                *velocity = point(Vec3::new(0.0, -10.0, 0.0));
                *position = point(Vec3::new(0.0, 0.1, 0.0));
            }
            a
        };
        e.max_particles = 1;
        let lib = Arc::new(Library::new(&gz_fmt_xr::Particles {
            effects: vec![e],
            groups: vec![],
            encoding: gz_fmt_xr::Encoding::default(),
        }));
        let mut s = lib.create("c\\drop").unwrap();
        s.play();
        s.on_frame(33, Some(&Floor));
        let p = s.particles()[0];
        // it moved 0.33 m down through the floor; the bounce sends it up at half the speed and
        // puts it a step from where it was
        assert!((p.vel - Vec3::new(0.0, 5.0, 0.0)).abs().max_element() < 1e-4, "{:?}", p.vel);
        assert!((p.pos - Vec3::new(0.0, 0.1 + 5.0 * 0.033, 0.0)).abs().max_element() < 1e-4, "{:?}", p.pos);
    }
}
