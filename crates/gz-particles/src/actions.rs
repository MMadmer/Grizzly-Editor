//! Every action of `particle_actions_collection.cpp`, run over an effect's particles. An action
//! keeps its values as loaded (the "local" ones) and the copies `Transform` carries into the
//! world; `execute` reads the world copies.

use glam::{Mat4, Vec3};
use gz_fmt_xr::{Action as Stored, ActionKind, ActionParams};

use crate::domain::{Domain, kind as dk, normalize_safe};
use crate::effect::{Particle, Store, argb_f};
use crate::noise::Noise;
use crate::rand::Rngs;

/// `P_MAXFLOAT`.
const P_MAXFLOAT: f32 = 1.0e16;
/// xrCore's `EPS` and `EPS_S`.
pub(crate) const EPS: f32 = 0.000_010_0;
pub(crate) const EPS_S: f32 = 0.000_000_1;

/// `ParticleAction::ALLOW_ROTATE`.
pub(crate) const ALLOW_ROTATE: u32 = 1 << 1;
/// `PASource` flags.
const SOURCE_SINGLE_SIZE: u32 = 1 << 29;
const SOURCE_SILENT: u32 = 1 << 30;

/// `v / s` as `pVector::operator/` does it: times the reciprocal.
fn div(v: Vec3, s: f32) -> Vec3 {
    v * (1.0 / s)
}

/// What an action needs besides the particles.
pub(crate) struct Ctx<'a> {
    pub rng: &'a mut Rngs,
    pub noise: &'a Noise,
}

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Avoid {
        local: Domain,
        world: Domain,
        look_ahead: f32,
        magnitude: f32,
        epsilon: f32,
    },
    Bounce {
        local: Domain,
        world: Domain,
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
        local: Vec3,
        center: Vec3,
        velocity: f32,
        magnitude: f32,
        stdev: f32,
        age: f32,
        epsilon: f32,
    },
    Follow {
        magnitude: f32,
        epsilon: f32,
        max_radius: f32,
    },
    Gravitate {
        magnitude: f32,
        epsilon: f32,
        max_radius: f32,
    },
    Gravity {
        direction: Vec3,
    },
    Jet {
        center_l: Vec3,
        acc_l: Domain,
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
    MatchVelocity {
        magnitude: f32,
        epsilon: f32,
        max_radius: f32,
    },
    Move,
    OrbitLine {
        p_l: Vec3,
        axis_l: Vec3,
        p: Vec3,
        axis: Vec3,
        magnitude: f32,
        epsilon: f32,
        max_radius: f32,
    },
    OrbitPoint {
        local: Vec3,
        center: Vec3,
        magnitude: f32,
        epsilon: f32,
        max_radius: f32,
    },
    RandomAccel {
        local: Domain,
        world: Domain,
    },
    RandomDisplace {
        local: Domain,
        world: Domain,
    },
    RandomVelocity {
        local: Domain,
        world: Domain,
    },
    Restore {
        time_left: f32,
    },
    Scatter {
        local: Vec3,
        center: Vec3,
        magnitude: f32,
        epsilon: f32,
        max_radius: f32,
    },
    Sink {
        kill_inside: u32,
        local: Domain,
        world: Domain,
    },
    SinkVelocity {
        kill_inside: u32,
        local: Domain,
        world: Domain,
    },
    Source(Box<SourceOp>),
    SpeedLimit {
        min_speed: f32,
        max_speed: f32,
    },
    TargetColor {
        color: Vec3,
        alpha: f32,
        scale: f32,
        time_from: f32,
        time_to: f32,
    },
    TargetSize {
        size: Vec3,
        scale: Vec3,
    },
    TargetRotate {
        rot: Vec3,
        scale: f32,
    },
    TargetVelocity {
        local: Vec3,
        velocity: Vec3,
        scale: f32,
    },
    Vortex {
        center_l: Vec3,
        axis_l: Vec3,
        center: Vec3,
        axis: Vec3,
        magnitude: f32,
        epsilon: f32,
        max_radius: f32,
    },
    Turbulence {
        frequency: f32,
        octaves: i32,
        magnitude: f32,
        epsilon: f32,
        offset: Vec3,
        age: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct SourceOp {
    position_l: Domain,
    velocity_l: Domain,
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
}

/// One action of an effect's list, as the engine runs it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Action {
    pub flags: u32,
    op: Op,
}

impl Action {
    /// `CreateAction` + `Load`: the local copies and the world copies start the same.
    pub(crate) fn new(a: &Stored) -> Self {
        use ActionParams as P;
        let op = match (&a.kind, &a.params) {
            (_, P::Avoid { position, look_ahead, magnitude, epsilon }) => {
                let d = Domain::from(position);
                Op::Avoid {
                    local: d,
                    world: d,
                    look_ahead: *look_ahead,
                    magnitude: *magnitude,
                    epsilon: *epsilon,
                }
            }
            (_, P::Bounce { position, one_minus_friction, resilience, cutoff_sqr }) => {
                let d = Domain::from(position);
                Op::Bounce {
                    local: d,
                    world: d,
                    one_minus_friction: *one_minus_friction,
                    resilience: *resilience,
                    cutoff_sqr: *cutoff_sqr,
                }
            }
            (_, P::CopyVertexB { copy_pos }) => Op::CopyVertexB { copy_pos: *copy_pos },
            (_, P::Damping { damping, vlow_sqr, vhigh_sqr }) => {
                Op::Damping { damping: *damping, vlow_sqr: *vlow_sqr, vhigh_sqr: *vhigh_sqr }
            }
            (_, P::Explosion { center, velocity, magnitude, stdev, age, epsilon }) => Op::Explosion {
                local: *center,
                center: *center,
                velocity: *velocity,
                magnitude: *magnitude,
                stdev: *stdev,
                age: *age,
                epsilon: *epsilon,
            },
            (ActionKind::Follow, P::Field { magnitude, epsilon, max_radius }) => {
                Op::Follow { magnitude: *magnitude, epsilon: *epsilon, max_radius: *max_radius }
            }
            (ActionKind::Gravitate, P::Field { magnitude, epsilon, max_radius }) => {
                Op::Gravitate { magnitude: *magnitude, epsilon: *epsilon, max_radius: *max_radius }
            }
            (_, P::Field { magnitude, epsilon, max_radius }) => {
                Op::MatchVelocity { magnitude: *magnitude, epsilon: *epsilon, max_radius: *max_radius }
            }
            (_, P::Gravity { direction }) => Op::Gravity { direction: *direction },
            (_, P::Jet { center, acc, magnitude, epsilon, max_radius }) => {
                let d = Domain::from(acc);
                Op::Jet {
                    center_l: *center,
                    acc_l: d,
                    center: *center,
                    acc: d,
                    magnitude: *magnitude,
                    epsilon: *epsilon,
                    max_radius: *max_radius,
                }
            }
            (_, P::KillOld { age_limit, kill_less_than }) => {
                Op::KillOld { age_limit: *age_limit, kill_less_than: *kill_less_than }
            }
            (_, P::Move) => Op::Move,
            (ActionKind::Vortex, P::Axis { point, axis, magnitude, epsilon, max_radius }) => Op::Vortex {
                center_l: *point,
                axis_l: *axis,
                center: *point,
                axis: *axis,
                magnitude: *magnitude,
                epsilon: *epsilon,
                max_radius: *max_radius,
            },
            (_, P::Axis { point, axis, magnitude, epsilon, max_radius }) => Op::OrbitLine {
                p_l: *point,
                axis_l: *axis,
                p: *point,
                axis: *axis,
                magnitude: *magnitude,
                epsilon: *epsilon,
                max_radius: *max_radius,
            },
            (ActionKind::Scatter, P::Point { center, magnitude, epsilon, max_radius }) => Op::Scatter {
                local: *center,
                center: *center,
                magnitude: *magnitude,
                epsilon: *epsilon,
                max_radius: *max_radius,
            },
            (_, P::Point { center, magnitude, epsilon, max_radius }) => Op::OrbitPoint {
                local: *center,
                center: *center,
                magnitude: *magnitude,
                epsilon: *epsilon,
                max_radius: *max_radius,
            },
            (ActionKind::RandomAccel, P::Random { domain }) => {
                let d = Domain::from(domain);
                Op::RandomAccel { local: d, world: d }
            }
            (ActionKind::RandomDisplace, P::Random { domain }) => {
                let d = Domain::from(domain);
                Op::RandomDisplace { local: d, world: d }
            }
            (_, P::Random { domain }) => {
                let d = Domain::from(domain);
                Op::RandomVelocity { local: d, world: d }
            }
            (_, P::Restore { time_left }) => Op::Restore { time_left: *time_left },
            (ActionKind::SinkVelocity, P::Sink { kill_inside, domain }) => {
                let d = Domain::from(domain);
                Op::SinkVelocity { kill_inside: *kill_inside, local: d, world: d }
            }
            (_, P::Sink { kill_inside, domain }) => {
                let d = Domain::from(domain);
                Op::Sink { kill_inside: *kill_inside, local: d, world: d }
            }
            (
                _,
                P::Source {
                    position,
                    velocity,
                    rot,
                    size,
                    color,
                    alpha,
                    particle_rate,
                    age,
                    age_sigma,
                    parent_vel,
                    parent_motion,
                },
            ) => {
                let (p, v) = (Domain::from(position), Domain::from(velocity));
                Op::Source(Box::new(SourceOp {
                    position_l: p,
                    velocity_l: v,
                    position: p,
                    velocity: v,
                    rot: Domain::from(rot),
                    size: Domain::from(size),
                    color: Domain::from(color),
                    alpha: *alpha,
                    particle_rate: *particle_rate,
                    age: *age,
                    age_sigma: *age_sigma,
                    parent_vel: *parent_vel,
                    parent_motion: *parent_motion,
                }))
            }
            (_, P::SpeedLimit { min_speed, max_speed }) => {
                Op::SpeedLimit { min_speed: *min_speed, max_speed: *max_speed }
            }
            (_, P::TargetColor { color, alpha, scale, time_from, time_to, .. }) => Op::TargetColor {
                color: *color,
                alpha: *alpha,
                scale: *scale,
                time_from: *time_from,
                time_to: *time_to,
            },
            (_, P::TargetSize { size, scale }) => Op::TargetSize { size: *size, scale: *scale },
            // TargetRotateD is made as a TargetRotate, TargetVelocityD as a TargetVelocity
            (ActionKind::TargetRotate | ActionKind::TargetRotateD, P::Target { value, scale }) => {
                Op::TargetRotate { rot: *value, scale: *scale }
            }
            (_, P::Target { value, scale }) => {
                Op::TargetVelocity { local: *value, velocity: *value, scale: *scale }
            }
            (_, P::Turbulence { frequency, octaves, magnitude, epsilon, offset }) => Op::Turbulence {
                frequency: *frequency,
                octaves: *octaves,
                magnitude: *magnitude,
                epsilon: *epsilon,
                offset: *offset,
                age: 0.0,
            },
        };
        Self { flags: a.flags, op }
    }

    /// `PlayEffect`: sources speak again, explosions and turbulence start over.
    pub(crate) fn on_play(&mut self) {
        match &mut self.op {
            Op::Source(_) => self.flags &= !SOURCE_SILENT,
            Op::Explosion { age, .. } | Op::Turbulence { age, .. } => *age = 0.0,
            _ => {}
        }
    }

    /// `StopEffect`: sources fall silent.
    pub(crate) fn on_stop(&mut self) {
        if matches!(self.op, Op::Source(_)) {
            self.flags |= SOURCE_SILENT;
        }
    }

    /// `CParticleManager::Transform`: the world copies from the local ones; a source's parent
    /// velocity from the emitter's.
    pub(crate) fn transform(&mut self, full: &Mat4, vel: Vec3) {
        let m = if self.flags & ALLOW_ROTATE != 0 {
            *full
        } else {
            Mat4::from_translation(full.w_axis.truncate())
        };
        match &mut self.op {
            Op::Avoid { local, world, .. }
            | Op::Bounce { local, world, .. }
            | Op::Sink { local, world, .. } => {
                *world = Domain::transformed(local, &m);
            }
            Op::Explosion { local, center, .. }
            | Op::OrbitPoint { local, center, .. }
            | Op::Scatter { local, center, .. } => *center = m.transform_point3(*local),
            Op::Jet { center_l, acc_l, center, acc, .. } => {
                *center = m.transform_point3(*center_l);
                *acc = Domain::transformed_dir(acc_l, &m);
            }
            Op::OrbitLine { p_l, axis_l, p, axis, .. } => {
                *p = m.transform_point3(*p_l);
                *axis = m.transform_vector3(*axis_l);
            }
            Op::Vortex { center_l, axis_l, center, axis, .. } => {
                *center = m.transform_point3(*center_l);
                *axis = m.transform_vector3(*axis_l);
            }
            Op::RandomAccel { local, world }
            | Op::RandomDisplace { local, world }
            | Op::RandomVelocity { local, world }
            | Op::SinkVelocity { local, world, .. } => *world = Domain::transformed_dir(local, &m),
            Op::Source(s) => {
                s.position = Domain::transformed(&s.position_l, &m);
                s.velocity = Domain::transformed_dir(&s.velocity_l, &m);
                s.parent_vel = vel * s.parent_motion;
            }
            Op::TargetVelocity { local, velocity, .. } => *velocity = m.transform_vector3(*local),
            _ => {}
        }
    }

    /// `Execute`: one step of `dt` seconds. `tm_max` is what `KillOld` says the oldest age is.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn execute(&mut self, e: &mut Store, dt: f32, tm_max: &mut f32, cx: &mut Ctx<'_>) {
        let flags = self.flags;
        match &mut self.op {
            Op::Avoid { world, look_ahead, magnitude, epsilon, .. } => {
                avoid(e, world, *look_ahead, *magnitude, *epsilon, dt);
            }
            Op::Bounce { world, one_minus_friction, resilience, cutoff_sqr, .. } => {
                bounce(e, world, *one_minus_friction, *resilience, *cutoff_sqr, dt, cx.rng);
            }
            Op::CopyVertexB { copy_pos } => {
                if *copy_pos != 0 {
                    for m in &mut e.list {
                        m.pos_b = m.pos;
                    }
                }
            }
            Op::Damping { damping, vlow_sqr, vhigh_sqr } => {
                let scale = Vec3::ONE - (Vec3::ONE - *damping) * dt;
                for m in &mut e.list {
                    let v = m.vel.length_squared();
                    if v >= *vlow_sqr && v <= *vhigh_sqr {
                        m.vel *= scale;
                    }
                }
            }
            Op::Explosion { center, velocity, magnitude, stdev, age, epsilon, .. } => {
                let radius = *velocity * *age;
                let magdt = *magnitude * dt;
                let one_over_sigma = 1.0 / *stdev;
                let inexp = -0.5 * one_over_sigma * one_over_sigma;
                let outexp = (1.0 / 2.506_628_3) * one_over_sigma;
                for m in &mut e.list {
                    let dir = m.pos - *center;
                    let dist_sqr = dir.length_squared();
                    let dist = dist_sqr.sqrt();
                    let wave = (radius - dist) * (radius - dist);
                    let gd = (wave * inexp).exp() * outexp;
                    m.vel += dir * (gd * magdt / ((dist + EPS) * (dist_sqr + *epsilon)));
                }
                *age += dt;
            }
            Op::Follow { magnitude, epsilon, max_radius } => {
                let magdt = *magnitude * dt;
                let max_sqr = *max_radius * *max_radius;
                let n = e.list.len();
                for i in 0..n.saturating_sub(1) {
                    let next = e.list[i + 1].pos;
                    let m = &mut e.list[i];
                    let to = next - m.pos;
                    let len_sqr = to.length_squared();
                    if max_sqr >= P_MAXFLOAT || len_sqr < max_sqr {
                        m.vel += to * (magdt / (len_sqr.sqrt() * (len_sqr + *epsilon)));
                    }
                }
            }
            Op::Gravitate { magnitude, epsilon, max_radius } => {
                let magdt = *magnitude * dt;
                let max_sqr = *max_radius * *max_radius;
                let n = e.list.len();
                for i in 0..n {
                    for j in i + 1..n {
                        let to = e.list[j].pos - e.list[i].pos;
                        let len_sqr = to.length_squared() + EPS_S;
                        if max_sqr >= P_MAXFLOAT || len_sqr < max_sqr {
                            let acc = to * (magdt / (len_sqr.sqrt() * (len_sqr + *epsilon)));
                            e.list[i].vel += acc;
                            e.list[j].vel -= acc;
                        }
                    }
                }
            }
            Op::Gravity { direction } => {
                let d = *direction * dt;
                for m in &mut e.list {
                    m.vel += d;
                }
            }
            Op::Jet { center, acc, magnitude, epsilon, max_radius, .. } => {
                let magdt = *magnitude * dt;
                let max_sqr = *max_radius * *max_radius;
                for m in &mut e.list {
                    let r = (m.pos - *center).length_squared();
                    if max_sqr >= P_MAXFLOAT || r < max_sqr {
                        let a = acc.generate(cx.rng);
                        m.vel += a * (magdt / (r + *epsilon));
                    }
                }
            }
            Op::KillOld { age_limit, kill_less_than } => {
                *tm_max = *age_limit;
                for i in (0..e.list.len()).rev() {
                    let younger = u32::from(e.list.get(i).is_some_and(|m| m.age < *age_limit));
                    if younger ^ *kill_less_than == 0 {
                        e.remove(i);
                    }
                }
            }
            Op::MatchVelocity { magnitude, epsilon, max_radius } => {
                let magdt = *magnitude * dt;
                let max_sqr = *max_radius * *max_radius;
                let n = e.list.len();
                for i in 0..n {
                    for j in i + 1..n {
                        let to = e.list[j].pos - e.list[i].pos;
                        let len_sqr = to.length_squared();
                        if max_sqr >= P_MAXFLOAT || len_sqr < max_sqr {
                            let acc = e.list[j].vel * (magdt / (len_sqr + *epsilon));
                            e.list[i].vel += acc;
                            e.list[j].vel -= acc;
                        }
                    }
                }
            }
            Op::Move => {
                for m in &mut e.list {
                    m.age += dt;
                    m.pos_b = m.pos;
                    m.pos += m.vel * dt;
                }
            }
            Op::OrbitLine { p, axis, magnitude, epsilon, max_radius, .. } => {
                let magdt = *magnitude * dt;
                let max_sqr = *max_radius * *max_radius;
                for m in &mut e.list {
                    let f = m.pos - *p;
                    let w = *axis * f.dot(*axis);
                    let into = w - f;
                    let r = into.length_squared();
                    if max_sqr >= P_MAXFLOAT || r < max_sqr {
                        m.vel += into * (magdt / (r.sqrt() + (r + *epsilon)));
                    }
                }
            }
            Op::OrbitPoint { center, magnitude, epsilon, max_radius, .. } => {
                let magdt = *magnitude * dt;
                let max_sqr = *max_radius * *max_radius;
                for m in &mut e.list {
                    let dir = *center - m.pos;
                    let r = dir.length_squared();
                    if max_sqr >= P_MAXFLOAT || r < max_sqr {
                        m.vel += dir * (magdt / (r.sqrt() + (r + *epsilon)));
                    }
                }
            }
            Op::RandomAccel { world, .. } => {
                for m in &mut e.list {
                    let a = world.generate(cx.rng);
                    m.vel += a * dt;
                }
            }
            Op::RandomDisplace { world, .. } => {
                for m in &mut e.list {
                    let d = world.generate(cx.rng);
                    m.pos += d * dt;
                }
            }
            Op::RandomVelocity { world, .. } => {
                for m in &mut e.list {
                    m.vel = world.generate(cx.rng);
                }
            }
            Op::Restore { time_left } => {
                if *time_left <= 0.0 {
                    for m in &mut e.list {
                        m.pos = m.pos_b;
                        m.vel = Vec3::ZERO;
                    }
                } else {
                    let t = *time_left;
                    let dt_sqr = dt * dt;
                    let t_sqr_inv_2dt = dt * 2.0 / (t * t);
                    let t_cub_inv_3dt_sqr = dt_sqr * 3.0 / (t * t * t);
                    for m in &mut e.list {
                        for a in 0..3 {
                            let (v, pb, p) = (m.vel[a], m.pos_b[a], m.pos[a]);
                            let b = (-2.0 * t * v + 3.0 * pb - 3.0 * p) * t_sqr_inv_2dt;
                            let aa = (t * v - pb - pb + p + p) * t_cub_inv_3dt_sqr;
                            m.vel[a] += aa + b;
                        }
                    }
                }
                *time_left -= dt;
            }
            Op::Scatter { center, magnitude, epsilon, max_radius, .. } => {
                let magdt = *magnitude * dt;
                let max_sqr = *max_radius * *max_radius;
                for m in &mut e.list {
                    let dir = m.pos - *center;
                    let r = dir.length_squared();
                    if max_sqr >= P_MAXFLOAT || r < max_sqr {
                        let a = div(dir, r.sqrt());
                        m.vel += a * (magdt / (r + *epsilon));
                    }
                }
            }
            Op::Sink { kill_inside, world, .. } => {
                for i in (0..e.list.len()).rev() {
                    let Some(pos) = e.list.get(i).map(|m| m.pos) else { continue };
                    if u32::from(world.within(pos, cx.rng)) ^ *kill_inside == 0 {
                        e.remove(i);
                    }
                }
            }
            Op::SinkVelocity { kill_inside, world, .. } => {
                for i in (0..e.list.len()).rev() {
                    let Some(vel) = e.list.get(i).map(|m| m.vel) else { continue };
                    if u32::from(world.within(vel, cx.rng)) ^ *kill_inside == 0 {
                        e.remove(i);
                    }
                }
            }
            Op::Source(s) => source(s, flags, e, dt, cx.rng),
            Op::SpeedLimit { min_speed, max_speed } => {
                let (min_sqr, max_sqr) = (*min_speed * *min_speed, *max_speed * *max_speed);
                for m in &mut e.list {
                    let s = m.vel.length_squared();
                    if s < min_sqr && s != 0.0 {
                        m.vel *= *min_speed / s.sqrt();
                    } else if s > max_sqr {
                        m.vel *= *max_speed / s.sqrt();
                    }
                }
            }
            Op::TargetColor { color, alpha, scale, time_from, time_to } => {
                let k = *scale * dt;
                for m in &mut e.list {
                    if m.age < *time_from * *tm_max || m.age > *time_to * *tm_max {
                        continue;
                    }
                    let c = unpack(m.color);
                    m.color = argb_f(
                        c[3] + (*alpha - c[3]) * k,
                        c[0] + (color.x - c[0]) * k,
                        c[1] + (color.y - c[1]) * k,
                        c[2] + (color.z - c[2]) * k,
                    );
                }
            }
            Op::TargetSize { size, scale } => {
                let k = *scale * dt;
                for m in &mut e.list {
                    m.size += (*size - m.size) * k;
                }
            }
            Op::TargetRotate { rot, scale } => {
                let k = *scale * dt;
                let r = rot.abs();
                for m in &mut e.list {
                    // oracle: the engine scales all three axes by the x axis's sign
                    let sign = if m.rot.x >= 0.0 { k } else { -k };
                    m.rot += (r - m.rot.abs()) * sign;
                }
            }
            Op::TargetVelocity { velocity, scale, .. } => {
                let k = *scale * dt;
                for m in &mut e.list {
                    m.vel += (*velocity - m.vel) * k;
                }
            }
            Op::Vortex { center, axis, magnitude, epsilon, max_radius, .. } => {
                let magdt = *magnitude * dt;
                let max_sqr = *max_radius * *max_radius;
                for m in &mut e.list {
                    let offset = m.pos - *center;
                    let r_sqr = offset.length_squared();
                    if max_sqr < P_MAXFLOAT && r_sqr > max_sqr {
                        continue;
                    }
                    let r = r_sqr.sqrt();
                    let off_norm = div(offset, r);
                    let proj = off_norm.dot(*axis);
                    let w = *axis * proj;
                    let u = off_norm - w;
                    let v = axis.cross(u);
                    let theta = magdt / (r_sqr + *epsilon);
                    let (s, c) = theta.sin_cos();
                    m.pos = (u * c + v * s + w) * r + *center;
                }
            }
            Op::Turbulence { frequency, octaves, magnitude, epsilon, offset, age } => {
                *age += dt;
                for m in &mut e.list {
                    let pv = m.pos + *offset * *age;
                    let d = cx.noise.fractalsum3(pv, *frequency, *octaves);
                    let dx = (cx.noise.fractalsum3(pv + Vec3::new(*epsilon, 0.0, 0.0), *frequency, *octaves)
                        - d)
                        * *magnitude;
                    let dy = (cx.noise.fractalsum3(pv + Vec3::new(0.0, *epsilon, 0.0), *frequency, *octaves)
                        - d)
                        * *magnitude;
                    let dz = (cx.noise.fractalsum3(pv + Vec3::new(0.0, 0.0, *epsilon), *frequency, *octaves)
                        - d)
                        * *magnitude;
                    let before = m.vel.length();
                    m.vel += Vec3::new(dx, dy, dz);
                    let now = m.vel.length();
                    m.vel *= before / now;
                }
            }
        }
    }
}

/// `Fcolor::set(u32)`: r, g, b, a over 0..1.
fn unpack(c: u32) -> [f32; 4] {
    let f = 1.0f32 / 255.0;
    [
        f * ((c >> 16) & 0xff) as f32,
        f * ((c >> 8) & 0xff) as f32,
        f * (c & 0xff) as f32,
        f * ((c >> 24) & 0xff) as f32,
    ]
}

/// The inverse of a plane basis `u`, `v` (Avoid's and Bounce's `s1`, `s2`).
fn plane_inverse(u: Vec3, v: Vec3) -> (Vec3, Vec3) {
    let wx = u.y * v.z - u.z * v.y;
    let wy = u.z * v.x - u.x * v.z;
    let wz = u.x * v.y - u.y * v.x;
    let det = 1.0
        / (wz * u.x * v.y - wz * u.y * v.x - u.z * wx * v.y - u.x * v.z * wy
            + v.z * wx * u.y
            + u.z * v.x * wy);
    let s1 = Vec3::new(v.y * wz - v.z * wy, v.z * wx - v.x * wz, v.x * wy - v.y * wx) * det;
    let s2 = Vec3::new(u.y * wz - u.z * wy, u.z * wx - u.x * wz, u.x * wy - u.y * wx) * -det;
    (s1, s2)
}

/// Blends a way out `s` into the velocity, keeping its speed.
fn steer(m: &mut Particle, s: Vec3, magdt: f32, t: f32, epsilon: f32) {
    let vm = m.vel.length();
    let vn = div(m.vel, vm);
    let tmp = s * (magdt / (t * t + epsilon)) + vn;
    m.vel = tmp * (vm / tmp.length());
}

fn avoid(e: &mut Store, d: &Domain, look_ahead: f32, magnitude: f32, epsilon: f32, dt: f32) {
    let magdt = magnitude * dt;
    match d.kind {
        dk::PLANE => {
            for m in &mut e.list {
                let dist = m.pos.dot(d.p2) + d.radius1;
                if look_ahead >= P_MAXFLOAT || dist < look_ahead {
                    let vm = m.vel.length();
                    let vn = div(m.vel, vm);
                    let tmp = d.p2 * (magdt / (dist * dist + epsilon)) + vn;
                    m.vel = tmp * (vm / tmp.length());
                }
            }
        }
        dk::RECTANGLE | dk::TRIANGLE => {
            let (u, v) = (d.u, d.v);
            let un = div(u, d.radius1_sqr);
            let vn = div(v, d.radius2_sqr);
            let fnn = normalize_safe(v - u);
            let (s1, s2) = plane_inverse(u, v);
            for m in &mut e.list {
                let next = m.pos + m.vel * dt * look_ahead;
                let old = m.pos.dot(d.p2) + d.radius1;
                let new = next.dot(d.p2) + d.radius1;
                if old * new >= 0.0 {
                    continue;
                }
                let nv = d.p2.dot(m.vel);
                let t = -old / nv;
                let hit = m.pos + m.vel * t;
                let offset = hit - d.p1;
                let (up, vp) = (offset.dot(s1), offset.dot(s2));
                let s = if d.kind == dk::RECTANGLE {
                    if up < 0.0 || vp < 0.0 || up > 1.0 || vp > 1.0 {
                        continue;
                    }
                    let uofs = un * un.dot(offset) - offset;
                    let vofs = vn * vn.dot(offset) - offset;
                    let foffset = (u + v) - offset;
                    let fofs = un * un.dot(foffset) - foffset;
                    // oracle: the fourth edge repeats the third one
                    let gofs = un * un.dot(foffset) - foffset;
                    let (ud, vd, fd, gd) = (
                        uofs.length_squared(),
                        vofs.length_squared(),
                        fofs.length_squared(),
                        gofs.length_squared(),
                    );
                    if ud <= vd && ud <= fd && ud <= gd {
                        uofs
                    } else if vd <= fd && vd <= gd {
                        vofs
                    } else if fd <= gd {
                        fofs
                    } else {
                        gofs
                    }
                } else {
                    if up < 0.0 || vp < 0.0 || up + vp > 1.0 {
                        continue;
                    }
                    let uofs = un * un.dot(offset) - offset;
                    let vofs = vn * vn.dot(offset) - offset;
                    let foffset = offset - u;
                    let fofs = fnn * fnn.dot(foffset) - foffset;
                    let (ud, vd, fd) = (uofs.length_squared(), vofs.length_squared(), fofs.length_squared());
                    if ud <= vd && ud <= fd {
                        uofs
                    } else if vd <= fd {
                        vofs
                    } else {
                        fofs
                    }
                };
                steer(m, normalize_safe(s), magdt, t, epsilon);
            }
        }
        dk::DISC => {
            let r1 = d.radius1 * d.radius1;
            let r2 = d.radius2 * d.radius2;
            for m in &mut e.list {
                let next = m.pos + m.vel * dt * look_ahead;
                // radius1_sqr holds the d of the disc's plane
                let old = m.pos.dot(d.p2) + d.radius1_sqr;
                let new = next.dot(d.p2) + d.radius1_sqr;
                if old * new >= 0.0 {
                    continue;
                }
                let t = -old / d.p2.dot(m.vel);
                let offset = (m.pos + m.vel * t) - d.p1;
                let rad = offset.length_squared();
                if rad > r1 || rad < r2 {
                    continue;
                }
                steer(m, normalize_safe(offset), magdt, t, epsilon);
            }
        }
        dk::SPHERE => {
            let r_sqr = d.radius1 * d.radius1;
            for m in &mut e.list {
                let vm = m.vel.length();
                let vn = div(m.vel, vm);
                let l = d.p1 - m.pos;
                let v = l.dot(vn);
                let disc = r_sqr - l.dot(l) + v * v;
                if disc < 0.0 {
                    continue;
                }
                let t = v - disc.sqrt();
                if t < 0.0 || t > vm * look_ahead {
                    continue;
                }
                let c = normalize_safe(vn.cross(l));
                let s = vn.cross(c);
                let tmp = s * (magdt / (t * t + epsilon)) + vn;
                m.vel = tmp * (vm / tmp.length());
            }
        }
        _ => {}
    }
}

/// The velocity heading out of a surface of normal `n`.
fn bounce_vel(vel: Vec3, n: Vec3, nmag: f32, omf: f32, res: f32, cutoff: f32) -> Vec3 {
    let vn = n * nmag;
    let vt = vel - vn;
    if vt.length_squared() <= cutoff { vt - vn * res } else { vt * omf - vn * res }
}

#[allow(clippy::too_many_arguments)]
fn bounce(e: &mut Store, d: &Domain, omf: f32, res: f32, cutoff: f32, dt: f32, rng: &mut Rngs) {
    match d.kind {
        dk::TRIANGLE | dk::RECTANGLE => {
            let (s1, s2) = plane_inverse(d.u, d.v);
            for m in &mut e.list {
                let next = m.pos + m.vel * dt;
                let old = m.pos.dot(d.p2) + d.radius1;
                let new = next.dot(d.p2) + d.radius1;
                if old * new >= 0.0 {
                    continue;
                }
                let nv = d.p2.dot(m.vel);
                let t = -old / nv;
                let offset = (m.pos + m.vel * t) - d.p1;
                let (up, vp) = (offset.dot(s1), offset.dot(s2));
                let outside = if d.kind == dk::TRIANGLE {
                    up < 0.0 || vp < 0.0 || up + vp > 1.0
                } else {
                    up < 0.0 || up > 1.0 || vp < 0.0 || vp > 1.0
                };
                if outside {
                    continue;
                }
                let nmag = if d.kind == dk::TRIANGLE { nv } else { m.vel.dot(d.p2) };
                m.vel = bounce_vel(m.vel, d.p2, nmag, omf, res, cutoff);
            }
        }
        dk::DISC => {
            let r1 = d.radius1 * d.radius1;
            let r2 = d.radius2 * d.radius2;
            for m in &mut e.list {
                let next = m.pos + m.vel * dt;
                let old = m.pos.dot(d.p2) + d.radius1_sqr;
                let new = next.dot(d.p2) + d.radius1_sqr;
                if old * new >= 0.0 {
                    continue;
                }
                let nv = d.p2.dot(m.vel);
                let t = -old / nv;
                let rad = ((m.pos + m.vel * t) - d.p1).length_squared();
                if rad > r1 || rad < r2 {
                    continue;
                }
                m.vel = bounce_vel(m.vel, d.p2, nv, omf, res, cutoff);
            }
        }
        dk::PLANE => {
            for m in &mut e.list {
                let next = m.pos + m.vel * dt;
                let old = m.pos.dot(d.p2) + d.radius1;
                let new = next.dot(d.p2) + d.radius1;
                if old * new >= 0.0 {
                    continue;
                }
                let nmag = m.vel.dot(d.p2);
                m.vel = bounce_vel(m.vel, d.p2, nmag, omf, res, cutoff);
            }
        }
        dk::SPHERE => {
            for m in &mut e.list {
                let next = m.pos + m.vel * dt;
                if !d.within(next, rng) {
                    continue;
                }
                let inside = d.within(m.pos, rng);
                let n = normalize_safe(m.pos - d.p1);
                let nmag = m.vel.dot(n);
                let vn = n * nmag;
                let vt = m.vel - vn;
                if inside {
                    if nmag < 0.0 {
                        m.vel = vt - vn;
                    }
                } else if vt.length_squared() <= cutoff {
                    m.vel = vt - vn * res;
                } else {
                    m.vel = vt * omf - vn * res;
                }
            }
        }
        _ => {}
    }
}

/// `PASource::Execute`: new particles at the rate, the fraction dithered in time.
fn source(s: &SourceOp, flags: u32, e: &mut Store, dt: f32, rng: &mut Rngs) {
    if flags & SOURCE_SILENT != 0 {
        return;
    }
    let mut rate = (s.particle_rate * dt).floor() as i32;
    if rng.drand() < s.particle_rate * dt - rate as f32 {
        rate += 1;
    }
    let count = e.list.len() as i64;
    if count + i64::from(rate) > i64::from(e.max) {
        rate = (i64::from(e.max) - count) as i32;
    }
    // with `flVertexB_tracks` (bit 31) posB is the position; without it the engine passes an
    // uninitialised posB - oracle: the position stands in (Move overwrites it before anything
    // reads it)
    for _ in 0..rate.max(0) {
        let pos = s.position.generate(rng);
        let mut siz = s.size.generate(rng);
        if flags & SOURCE_SINGLE_SIZE != 0 {
            siz = Vec3::splat(siz.x);
        }
        let rot = s.rot.generate(rng);
        let vel = s.velocity.generate(rng) + s.parent_vel;
        let col = s.color.generate(rng);
        let age = s.age + rng.nrand(s.age_sigma);
        let p = Particle {
            pos,
            pos_b: pos,
            vel,
            size: siz,
            rot,
            color: argb_f(s.alpha, col.x, col.y, col.z),
            age,
            frame: 0,
            flags: 0,
        };
        if !e.add(p, rng) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_fmt_xr::Domain as FDomain;

    fn store(n: usize) -> Store {
        let mut s = Store::new(64);
        for i in 0..n {
            s.list.push(Particle {
                pos: Vec3::new(i as f32, 1.0, 0.0),
                pos_b: Vec3::new(i as f32, 1.0, 0.0),
                vel: Vec3::new(1.0, 0.0, 0.0),
                size: Vec3::ONE,
                age: i as f32,
                color: 0xff80_8080,
                ..Particle::default()
            });
        }
        s
    }

    fn run(a: &Stored, s: &mut Store, dt: f32) -> f32 {
        let mut act = Action::new(a);
        let mut rng = Rngs::default();
        let noise = Noise::new();
        let mut tm = 1.0;
        act.execute(s, dt, &mut tm, &mut Ctx { rng: &mut rng, noise: &noise });
        tm
    }

    fn stored(kind: ActionKind, params: ActionParams) -> Stored {
        Stored { kind, flags: 0, params }
    }

    #[test]
    fn move_gravity_and_damping() {
        let mut s = store(2);
        run(
            &stored(ActionKind::Gravity, ActionParams::Gravity { direction: Vec3::new(0.0, -10.0, 0.0) }),
            &mut s,
            0.5,
        );
        assert_eq!(s.list[0].vel, Vec3::new(1.0, -5.0, 0.0));
        run(&stored(ActionKind::Move, ActionParams::Move), &mut s, 0.5);
        // posB is where it was, pos moved by vel * dt, age grew
        assert_eq!(s.list[1].pos_b, Vec3::new(1.0, 1.0, 0.0));
        assert_eq!(s.list[1].pos, Vec3::new(1.5, -1.5, 0.0));
        assert_eq!(s.list[1].age, 1.5);
        run(
            &stored(
                ActionKind::Damping,
                ActionParams::Damping { damping: Vec3::splat(0.5), vlow_sqr: 0.0, vhigh_sqr: 1e9 },
            ),
            &mut s,
            1.0,
        );
        // scale = 1 - (1 - 0.5) * 1
        assert_eq!(s.list[0].vel, Vec3::new(0.5, -2.5, 0.0));
    }

    #[test]
    fn kill_old_removes_by_age_and_sets_the_limit() {
        let mut s = store(5);
        let tm = run(
            &stored(ActionKind::KillOld, ActionParams::KillOld { age_limit: 2.5, kill_less_than: 0 }),
            &mut s,
            0.1,
        );
        assert_eq!(tm, 2.5);
        // ages 0, 1, 2 stay; 3 and 4 go
        let mut ages: Vec<f32> = s.list.iter().map(|m| m.age).collect();
        ages.sort_by(f32::total_cmp);
        assert_eq!(ages, vec![0.0, 1.0, 2.0]);
        let mut s = store(5);
        run(
            &stored(ActionKind::KillOld, ActionParams::KillOld { age_limit: 2.5, kill_less_than: 1 }),
            &mut s,
            0.1,
        );
        assert_eq!(s.list.len(), 2);
    }

    #[test]
    fn target_color_and_size() {
        let mut s = store(1);
        run(
            &stored(
                ActionKind::TargetColor,
                ActionParams::TargetColor {
                    color: Vec3::ONE,
                    alpha: 0.0,
                    scale: 1.0,
                    time_from: 0.0,
                    time_to: 1.0,
                    times_stored: 2,
                },
            ),
            &mut s,
            0.5,
        );
        // r = 128/255 + (1 - 128/255) * 0.5 = 0.75098 -> 191; a = 1 - 0.5 = 0.5 -> 127
        let c = s.list[0].color;
        assert_eq!((c >> 24) & 0xff, 127);
        assert_eq!((c >> 16) & 0xff, 191);
        let mut s = store(1);
        run(
            &stored(
                ActionKind::TargetSize,
                ActionParams::TargetSize { size: Vec3::splat(3.0), scale: Vec3::new(0.5, 1.0, 0.0) },
            ),
            &mut s,
            1.0,
        );
        assert_eq!(s.list[0].size, Vec3::new(2.0, 3.0, 1.0));
    }

    #[test]
    fn source_emits_at_its_rate_with_dithering() {
        let mut s = Store::new(10);
        let point = |p: Vec3| FDomain { kind: 0, p1: p, ..FDomain::default() };
        let a = stored(
            ActionKind::Source,
            ActionParams::Source {
                position: point(Vec3::new(1.0, 2.0, 3.0)),
                velocity: point(Vec3::Y),
                rot: point(Vec3::ZERO),
                size: point(Vec3::splat(0.5)),
                color: point(Vec3::ONE),
                alpha: 1.0,
                particle_rate: 25.0,
                age: 0.0,
                age_sigma: 0.0,
                parent_vel: Vec3::ZERO,
                parent_motion: 0.0,
            },
        );
        // 25 * 0.1 = 2.5: two, and a third when the first draw (41 / 32767) is under 0.5
        run(&a, &mut s, 0.1);
        assert_eq!(s.list.len(), 3);
        assert_eq!(s.list[0].pos, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(s.list[0].color, 0xffff_ffff);
        // no more than the effect holds
        run(&a, &mut s, 10.0);
        assert_eq!(s.list.len(), 10);
    }

    #[test]
    fn vortex_turns_round_the_axis_keeping_the_radius() {
        let mut s = store(3);
        run(
            &stored(
                ActionKind::Vortex,
                ActionParams::Axis {
                    point: Vec3::ZERO,
                    axis: Vec3::Y,
                    magnitude: 1.0,
                    epsilon: 0.0,
                    max_radius: 1e9,
                },
            ),
            &mut s,
            0.25,
        );
        let p = s.list[2].pos;
        assert!(((p.x * p.x + p.z * p.z).sqrt() - 2.0).abs() < 1e-4);
        assert!((p.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn transform_carries_world_copies() {
        let mut act = Action::new(&stored(
            ActionKind::OrbitPoint,
            ActionParams::Point { center: Vec3::X, magnitude: 1.0, epsilon: 0.0, max_radius: 1.0 },
        ));
        act.transform(&Mat4::from_translation(Vec3::new(0.0, 5.0, 0.0)), Vec3::ZERO);
        match &act.op {
            Op::OrbitPoint { center, .. } => assert_eq!(*center, Vec3::new(1.0, 5.0, 0.0)),
            _ => unreachable!(),
        }
        // without ALLOW_ROTATE only the translation counts
        let mut act = Action::new(&stored(ActionKind::Gravity, ActionParams::Gravity { direction: Vec3::Y }));
        act.transform(&Mat4::from_rotation_x(1.0), Vec3::ZERO);
        assert!(matches!(act.op, Op::Gravity { direction } if direction == Vec3::Y));
    }
}
