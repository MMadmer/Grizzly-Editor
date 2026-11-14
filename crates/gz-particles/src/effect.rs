//! One particle effect as the engine plays it: the particle store, the
//! playback of the effect (33 ms steps, the time limit, the deferred stop) and what
//! the effect definition adds to the actions - frame animation, collision and the sprites the render fills.

use std::sync::Arc;

use glam::{Mat4, Vec2, Vec3};
use smol_str::SmolStr;

use crate::actions::{Action, Ctx, EPS, EPS_S};
use crate::rand::Rngs;
use crate::system::{Collider, EffectDef, flags};
use crate::{STEP, STEP_MS};

/// xrCore's `EPS_L`.
const EPS_L: f32 = 0.001_000_0;

/// A particle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Particle {
    pub pos: Vec3,
    /// Where it was before the last move.
    pub pos_b: Vec3,
    pub vel: Vec3,
    pub size: Vec3,
    /// `rot.x` is the sprite's angle.
    pub rot: Vec3,
    /// `0xAARRGGBB`.
    pub color: u32,
    pub age: f32,
    /// The atlas frame times 255.
    pub frame: u16,
    /// [`Particle::ANIMATE_CCW`].
    pub flags: u16,
}

impl Particle {
    /// Plays its frames backwards.
    pub const ANIMATE_CCW: u16 = 1 << 0;
}

/// `color_argb_f`: each channel `floor(x * 255)` clamped to a byte.
pub(crate) fn argb_f(a: f32, r: f32, g: f32, b: f32) -> u32 {
    let q = |x: f32| ((x * 255.0).floor() as i32).clamp(0, 255) as u32;
    (q(a) << 24) | (q(r) << 16) | (q(g) << 8) | q(b)
}

/// What a store reports to a group about its particles (the birth and death callbacks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Event {
    Birth(Particle),
    /// The index the particle had: the group's related child of that index stops.
    Death(usize, Particle),
}

/// What a new particle gets from its effect's definition.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Birth {
    /// The frame count.
    pub random_frame: Option<i32>,
    /// The animated flag with random playback.
    pub random_playback: bool,
}

/// The particles and how many there may be.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Store {
    pub list: Vec<Particle>,
    pub max: u32,
    pub birth: Birth,
    /// Whether births and deaths are recorded for a group.
    pub record: bool,
    pub events: Vec<Event>,
}

impl Store {
    pub(crate) fn new(max: u32) -> Self {
        Self { max, ..Self::default() }
    }

    /// `Add`, the birth callback run on the particle before it is counted.
    pub(crate) fn add(&mut self, mut p: Particle, rng: &mut Rngs) -> bool {
        if self.list.len() as u64 >= u64::from(self.max) {
            return false;
        }
        if let Some(count) = self.birth.random_frame {
            p.frame = ((rng.random.rand_i_max(count) as f32 * 255.0).floor() as i32) as u16;
        }
        if self.birth.random_playback && rng.random.rand_i_max(2) != 0 {
            p.flags |= Particle::ANIMATE_CCW;
        }
        if self.record {
            self.events.push(Event::Birth(p));
        }
        self.list.push(p);
        true
    }

    /// `Remove`: the death callback, then the last particle takes the place.
    pub(crate) fn remove(&mut self, i: usize) {
        if i >= self.list.len() {
            return;
        }
        if self.record
            && let Some(p) = self.list.get(i)
        {
            self.events.push(Event::Death(i, *p));
        }
        self.list.swap_remove(i);
    }
}

/// How a sprite is turned (two forms).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    /// Facing the camera: its top and right are the camera's.
    Camera,
    /// `t` is the sprite's top, `r` its right.
    Axes { t: Vec3, r: Vec3 },
    /// Along a direction: its top is `dir`, its right `dir x camera direction`.
    Dir(Vec3),
}

/// Which faces the engine culls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cull {
    #[default]
    None,
    /// Clockwise faces go.
    Clockwise,
    /// `CULL_CCW`.
    CounterClockwise,
}

/// One particle as the renderer draws it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub pos: Vec3,
    /// Half its width and half its height.
    pub half: Vec2,
    /// Turned by this angle about its normal.
    pub angle: f32,
    /// `0xAARRGGBB`.
    pub color: u32,
    /// The atlas frame: left-top u, v, right-bottom u, v.
    pub uv: [f32; 4],
    pub align: Align,
}

/// The sprites of one effect: drawn with the effect's shader and texture.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteBatch {
    pub shader: SmolStr,
    pub texture: SmolStr,
    pub cull: Cull,
    /// The box of the particles grown by the largest one (`vis.box`).
    pub bounds: (Vec3, Vec3),
    pub sprites: Vec<Sprite>,
}

/// A particle effect.
#[derive(Clone, Debug)]
pub(crate) struct Effect {
    pub def: Option<Arc<EffectDef>>,
    actions: Vec<Action>,
    pub store: Store,
    playing: bool,
    deferred: bool,
    xform_on: bool,
    xform: Mat4,
    elapsed_limit: f32,
    mem_dt: i32,
    initial: Vec3,
    /// `vis.box`: none before the first frame.
    pub bounds: Option<(Vec3, Vec3)>,
}

impl Effect {
    /// A particle effect, compiled: the actions loaded, the birth rule set; not playing.
    pub(crate) fn new(def: Option<Arc<EffectDef>>) -> Self {
        let mut store = Store::new(1);
        let mut actions = Vec::new();
        let mut elapsed_limit = 0.0;
        if let Some(d) = &def {
            actions = d.actions.iter().map(Action::new).collect();
            store.max = d.max_particles;
            store.birth = Birth {
                random_frame: (d.flags & flags::RANDOM_FRAME != 0).then_some(d.frame.frame_count),
                random_playback: d.flags & flags::ANIMATED != 0 && d.flags & flags::RANDOM_PLAYBACK != 0,
            };
            if d.flags & flags::TIME_LIMIT != 0 {
                elapsed_limit = d.time_limit;
            }
        }
        Self {
            def,
            actions,
            store,
            playing: false,
            deferred: false,
            xform_on: false,
            xform: Mat4::IDENTITY,
            elapsed_limit,
            mem_dt: 0,
            initial: Vec3::ZERO,
            bounds: None,
        }
    }

    pub(crate) fn is_playing(&self) -> bool {
        self.playing
    }

    /// The emitter's own space, when it draws in it.
    pub(crate) fn xform(&self) -> Option<Mat4> {
        self.xform_on.then_some(self.xform)
    }

    /// No time limit.
    pub(crate) fn is_looped(&self) -> bool {
        self.def.as_ref().is_none_or(|d| d.flags & flags::TIME_LIMIT == 0)
    }

    pub(crate) fn play(&mut self) {
        self.deferred = false;
        self.playing = true;
        for a in &mut self.actions {
            a.on_play();
        }
    }

    /// `Stop`: deferred, the sources fall silent and the particles live out their lives;
    /// otherwise they are gone at once.
    pub(crate) fn stop(&mut self, deferred: bool) {
        for a in &mut self.actions {
            a.on_stop();
        }
        if deferred {
            self.deferred = true;
        } else {
            // StopEffect drops the particles without telling anybody
            self.store.list.clear();
            self.playing = false;
        }
    }

    /// Updates the parent.
    pub(crate) fn update_parent(&mut self, m: &Mat4, vel: Vec3, xform: bool) {
        self.xform_on = xform;
        if xform {
            self.xform = *m;
        } else {
            self.initial = m.w_axis.truncate();
            for a in &mut self.actions {
                a.transform(m, vel);
            }
        }
    }

    /// As many 33 ms steps as the time owes, three at most.
    pub(crate) fn on_frame(&mut self, dt_ms: u32, cx: &mut Ctx<'_>, collider: Option<&dyn Collider>) {
        let Some(def) = self.def.clone().filter(|_| self.playing) else {
            let e = Vec3::splat(EPS_L);
            self.bounds = Some((self.initial - e, self.initial + e));
            return;
        };
        self.mem_dt = self.mem_dt.saturating_add(i32::try_from(dt_ms).unwrap_or(i32::MAX));
        let step = STEP_MS as i32;
        let mut steps = 0;
        if self.mem_dt >= step {
            steps = (self.mem_dt / step).clamp(0, 3);
            self.mem_dt %= step;
        }
        for _ in 0..steps {
            if def.flags & flags::TIME_LIMIT != 0 && !self.deferred {
                self.elapsed_limit -= STEP;
                if self.elapsed_limit < 0.0 {
                    self.elapsed_limit = def.time_limit;
                    self.stop(true);
                    break;
                }
            }
            let mut kill_old_time = 1.0f32;
            for a in &mut self.actions {
                a.execute(&mut self.store, STEP, &mut kill_old_time, cx);
            }
            if def.flags & (flags::FRAMED | flags::ANIMATED) == (flags::FRAMED | flags::ANIMATED) {
                animate(&def, &mut self.store.list, STEP);
            }
            if def.flags & flags::COLLISION != 0 {
                self.collide(&def, STEP, collider);
            }
            if !self.store.list.is_empty() {
                let mut lo = Vec3::splat(f32::MAX);
                let mut hi = Vec3::splat(-f32::MAX);
                let mut size = 0.0f32;
                for m in &self.store.list {
                    lo = lo.min(m.pos);
                    hi = hi.max(m.pos);
                    size = size.max(m.size.x).max(m.size.y).max(m.size.z);
                }
                self.bounds = Some((lo - Vec3::splat(size), hi + Vec3::splat(size)));
            }
            if self.deferred && self.store.list.is_empty() {
                self.playing = false;
                self.deferred = false;
                break;
            }
        }
    }

    /// Each particle's last move tested against the surfaces, twice at
    /// most; a hit bounces it, or takes it away with the collision-delete flag.
    fn collide(&mut self, def: &EffectDef, dt: f32, collider: Option<&dyn Collider>) {
        let [omf, res, cutoff] = def.collide;
        for i in (0..self.store.list.len()).rev() {
            let mut picks = 0;
            while let Some(m) = self.store.list.get(i).copied() {
                let dir = m.pos - m.pos_b;
                let dist = dir.length();
                if dist < EPS {
                    if let Some(p) = self.store.list.get_mut(i) {
                        p.pos = p.pos_b;
                    }
                    break;
                }
                let dir = dir * (1.0 / dist);
                let Some((_point, n)) = collider.and_then(|c| c.ray_pick(m.pos_b, dir, dist)) else { break };
                picks += 1;
                if def.flags & flags::COLLISION_DEL != 0 {
                    self.store.remove(i);
                    break;
                }
                let nmag = m.vel.dot(n);
                let vn = n * nmag;
                let vt = m.vel - vn;
                let vel = if vt.length_squared() <= cutoff { vt - vn * res } else { vt * omf - vn * res };
                if let Some(p) = self.store.list.get_mut(i) {
                    p.vel = vel;
                    p.pos = p.pos_b + vel * dt;
                }
                if picks >= 2 {
                    break;
                }
            }
        }
    }

    /// The effect's sprites, when it is a sprite effect with particles.
    pub(crate) fn sprites(&self) -> Option<SpriteBatch> {
        let def = self.def.as_ref()?;
        if def.flags & flags::SPRITE == 0 || self.store.list.is_empty() {
            return None;
        }
        let framed = def.flags & flags::FRAMED != 0;
        let align = def.flags & flags::ALIGN_TO_PATH != 0;
        let velocity_scale = def.flags & flags::VELOCITY_SCALE != 0;
        let mut sprites = Vec::with_capacity(self.store.list.len());
        for m in &self.store.list {
            let (mut lt, mut rb) = (Vec2::ZERO, Vec2::ONE);
            if framed {
                (lt, rb) = def.frame.tc((f32::from(m.frame) / 255.0).floor() as i32);
            }
            let mut half = Vec2::new(m.size.x * 0.5, m.size.y * 0.5);
            if velocity_scale {
                let speed = m.vel.length();
                half.x += speed * def.velocity_scale.x;
                half.y += speed * def.velocity_scale.y;
            }
            let pos = if self.xform_on { self.xform.transform_point3(m.pos) } else { m.pos };
            let to_world = |v: Vec3| if self.xform_on { self.xform.transform_vector3(v) } else { v };
            let align = if align {
                let speed = m.vel.length();
                if speed < EPS_S && def.flags & flags::WORLD_ALIGN != 0 {
                    let r = def.ap_default_rotation;
                    let mm = hpb_rows(r.y, r.x, r.z);
                    Align::Axes { t: to_world(mm[2]), r: to_world(mm[0]) }
                } else if speed >= EPS_S && def.flags & flags::FACE_ALIGN != 0 {
                    let k = m.vel * (1.0 / speed);
                    let mut j = Vec3::Y;
                    if j.dot(k).abs() > 0.99 {
                        j = Vec3::Z;
                    }
                    let i = j.cross(k).normalize_or_zero();
                    let j = k.cross(i).normalize_or_zero();
                    Align::Axes { t: to_world(j), r: to_world(i) }
                } else {
                    let dir = if speed >= EPS_S {
                        m.vel * (1.0 / speed)
                    } else {
                        let r = def.ap_default_rotation;
                        dir_hp(-r.y, -r.x)
                    };
                    Align::Dir(to_world(dir))
                }
            } else {
                Align::Camera
            };
            sprites.push(Sprite {
                pos,
                half,
                angle: m.rot.x,
                color: m.color,
                uv: [lt.x, lt.y, rb.x, rb.y],
                align,
            });
        }
        let cull = if def.flags & flags::CULLING == 0 {
            Cull::None
        } else if def.flags & flags::CULL_CCW != 0 {
            Cull::CounterClockwise
        } else {
            Cull::Clockwise
        };
        let bounds = match self.bounds {
            Some((lo, hi)) if self.xform_on => {
                let (a, b) = (self.xform.transform_point3(lo), self.xform.transform_point3(hi));
                (a.min(b), a.max(b))
            }
            Some(b) => b,
            None => {
                let e = Vec3::splat(EPS_L);
                (self.initial - e, self.initial + e)
            }
        };
        Some(SpriteBatch { shader: def.shader.clone(), texture: def.texture.clone(), cull, bounds, sprites })
    }
}

/// Executes the frame animation.
fn animate(def: &EffectDef, list: &mut [Particle], dt: f32) {
    let k = def.frame.speed * dt;
    let count = def.frame.frame_count as f32;
    for m in list {
        let dir = if m.flags & Particle::ANIMATE_CCW != 0 { -1.0 } else { 1.0 };
        let mut f = f32::from(m.frame) / 255.0 + dir * k;
        if f > count {
            f -= count;
        }
        if f < 0.0 {
            f += count;
        }
        m.frame = ((f * 255.0).floor() as i32) as u16;
    }
}

/// The rows `i`, `j`, `k` of the heading-pitch-bank rotation (the XYZ form is the same with the angles in another order).
pub(crate) fn hpb_rows(h: f32, p: f32, b: f32) -> [Vec3; 3] {
    let (sh, ch) = h.sin_cos();
    let (sp, cp) = p.sin_cos();
    let (sb, cb) = b.sin_cos();
    let (cc, cs, sc, ss) = (ch * cb, ch * sb, sh * cb, sh * sb);
    [
        Vec3::new(cc - sp * ss, -cp * sb, sp * cs + sc),
        Vec3::new(sp * sc + cs, cp * cb, ss - sp * cc),
        Vec3::new(-cp * sh, sp, cp * ch),
    ]
}

/// The heading and pitch of a direction.
pub(crate) fn dir_hp(h: f32, p: f32) -> Vec3 {
    let (sh, ch) = h.sin_cos();
    let (sp, cp) = p.sin_cos();
    Vec3::new(-cp * sh, sp, cp * ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_quantise_like_the_engine() {
        // alpha first, then red, green (floor(127.5) = 127), blue
        assert_eq!(argb_f(1.0, 1.0, 0.5, 0.0), 0xffff_7f00);
        assert_eq!(argb_f(2.0, -1.0, 0.999, 1.0), 0xff00_feff);
    }

    #[test]
    fn the_store_swaps_the_last_in_and_reports() {
        let mut s = Store::new(3);
        s.record = true;
        let mut rng = Rngs::default();
        for i in 0..4 {
            let added = s.add(Particle { age: i as f32, ..Particle::default() }, &mut rng);
            assert_eq!(added, i < 3);
        }
        s.remove(0);
        assert_eq!(s.list.iter().map(|p| p.age).collect::<Vec<_>>(), vec![2.0, 1.0]);
        assert!(matches!(s.events.last(), Some(Event::Death(0, p)) if p.age == 0.0));
        assert_eq!(s.events.iter().filter(|e| matches!(e, Event::Birth(_))).count(), 3);
    }

    #[test]
    fn a_random_frame_comes_from_random() {
        let mut s = Store::new(4);
        s.birth.random_frame = Some(16);
        let mut rng = Rngs::default();
        s.add(Particle::default(), &mut rng);
        // randI(16) of the first draw 41 is 9: frame 9 * 255
        assert_eq!(s.list[0].frame, 9 * 255);
    }

    #[test]
    fn setxyz_rows() {
        // (-pi/2, 0, 0) through setXYZ: heading 0, pitch -pi/2
        let m = hpb_rows(0.0, -std::f32::consts::FRAC_PI_2, 0.0);
        assert!((m[2] - Vec3::new(0.0, -1.0, 0.0)).abs().max_element() < 1e-6);
        assert!((m[0] - Vec3::X).abs().max_element() < 1e-6);
        assert!((dir_hp(0.0, 0.0) - Vec3::Z).abs().max_element() < 1e-6);
    }
}
