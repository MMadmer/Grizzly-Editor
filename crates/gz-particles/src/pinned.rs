//! One pinned step of every action the module tests of `actions.rs` leave out, the numbers
//! worked out from the C++ arithmetic.

use glam::Vec3;
use gz_fmt_xr::{Action as Stored, ActionKind, ActionParams, Domain as FDomain};

use crate::actions::{Action, Ctx};
use crate::domain::kind as dk;
use crate::effect::{Particle, Store};
use crate::noise::Noise;
use crate::rand::Rngs;

fn store(parts: &[(Vec3, Vec3)]) -> Store {
    let mut s = Store::new(64);
    for (pos, vel) in parts {
        s.list.push(Particle {
            pos: *pos,
            pos_b: *pos,
            vel: *vel,
            size: Vec3::ONE,
            color: 0xffff_ffff,
            ..Particle::default()
        });
    }
    s
}

fn step(kind: ActionKind, params: ActionParams, s: &mut Store, dt: f32) {
    let mut act = Action::new(&Stored { kind, flags: 0, params });
    let mut rng = Rngs::default();
    let noise = Noise::new();
    let mut tm = 1.0;
    act.execute(s, dt, &mut tm, &mut Ctx { rng: &mut rng, noise: &noise });
}

fn near(a: Vec3, b: Vec3) -> bool {
    (a - b).abs().max_element() < 1e-4
}

fn pt(p: Vec3) -> FDomain {
    FDomain { kind: dk::POINT, p1: p, ..FDomain::default() }
}

const X: Vec3 = Vec3::X;

#[test]
fn copy_vertex_b_and_restore() {
    let mut s = store(&[(Vec3::ONE, X)]);
    s.list[0].pos_b = Vec3::ONE;
    step(ActionKind::CopyVertexB, ActionParams::CopyVertexB { copy_pos: 1 }, &mut s, 1.0);
    assert_eq!(s.list[0].pos_b, Vec3::ONE);
    // time left 1, dt 1, v = 1, posB = pos: b = -2 * 2 = -4, a = 1 * 3 = 3, v = 1 + 3 - 4
    step(ActionKind::Restore, ActionParams::Restore { time_left: 1.0 }, &mut s, 1.0);
    assert!(near(s.list[0].vel, Vec3::ZERO));
    // with no time left the particle is put back and stopped
    let mut s = store(&[(Vec3::ONE, X)]);
    s.list[0].pos_b = Vec3::ZERO;
    step(ActionKind::Restore, ActionParams::Restore { time_left: 0.0 }, &mut s, 1.0);
    assert_eq!((s.list[0].pos, s.list[0].vel), (Vec3::ZERO, Vec3::ZERO));
}

#[test]
fn explosion_pushes_by_the_wave() {
    let mut s = store(&[(Vec3::new(1.0, 1.0, 0.0), X)]);
    step(
        ActionKind::Explosion,
        ActionParams::Explosion {
            center: Vec3::Y,
            velocity: 0.0,
            magnitude: 1.0,
            stdev: 1.0,
            age: 0.0,
            epsilon: 0.0,
        },
        &mut s,
        1.0,
    );
    // Gd = exp(-0.5) / sqrt(2 pi) = 0.241971, over (1 + EPS) * (1 + 0)
    assert!(near(s.list[0].vel, Vec3::new(1.0 + 0.241_971 / 1.000_01, 0.0, 0.0)));
}

#[test]
fn follow_gravitate_and_match_velocity() {
    let two = [(Vec3::ZERO, X), (X, X)];
    let field = ActionParams::Field { magnitude: 1.0, epsilon: 0.0, max_radius: 1e9 };
    let mut s = store(&two);
    step(ActionKind::Follow, field.clone(), &mut s, 1.0);
    // towards the next one by 1 / (1 * 1); the last follows nobody
    assert!(near(s.list[0].vel, Vec3::new(2.0, 0.0, 0.0)) && near(s.list[1].vel, X));
    let mut s = store(&two);
    step(ActionKind::Gravitate, field.clone(), &mut s, 1.0);
    assert!(near(s.list[0].vel, Vec3::new(2.0, 0.0, 0.0)) && near(s.list[1].vel, Vec3::ZERO));
    let mut s = store(&two);
    step(ActionKind::MatchVelocity, field, &mut s, 1.0);
    // the second's velocity over (1 + 0) is given to the first and taken from the second
    assert!(near(s.list[0].vel, Vec3::new(2.0, 0.0, 0.0)) && near(s.list[1].vel, Vec3::ZERO));
}

#[test]
fn jet_orbits_and_scatter() {
    let two = [(Vec3::Y, X), (Vec3::new(1.0, 1.0, 0.0), X)];
    let mut s = store(&two);
    step(
        ActionKind::Jet,
        ActionParams::Jet {
            center: Vec3::Y,
            acc: pt(Vec3::Y),
            magnitude: 1.0,
            epsilon: 1.0,
            max_radius: 1e9,
        },
        &mut s,
        1.0,
    );
    // the acceleration over r^2 + epsilon: 1 / 1 at the centre, 1 / 2 a metre away
    assert!(near(s.list[0].vel, Vec3::new(1.0, 1.0, 0.0)) && near(s.list[1].vel, Vec3::new(1.0, 0.5, 0.0)));
    let mut s = store(&two);
    step(
        ActionKind::OrbitLine,
        ActionParams::Axis {
            point: Vec3::ZERO,
            axis: Vec3::Y,
            magnitude: 1.0,
            epsilon: 0.0,
            max_radius: 1e9,
        },
        &mut s,
        1.0,
    );
    // into the line, (-1, 0, 0), over (r + r^2) = 2
    assert!(near(s.list[1].vel, Vec3::new(0.5, 0.0, 0.0)));
    let mut s = store(&two);
    step(
        ActionKind::OrbitPoint,
        ActionParams::Point { center: Vec3::Y, magnitude: 1.0, epsilon: 0.0, max_radius: 1e9 },
        &mut s,
        1.0,
    );
    assert!(near(s.list[1].vel, Vec3::new(0.5, 0.0, 0.0)));
    let mut s = store(&two);
    step(
        ActionKind::Scatter,
        ActionParams::Point { center: Vec3::Y, magnitude: 1.0, epsilon: 0.0, max_radius: 1e9 },
        &mut s,
        1.0,
    );
    // away from the centre, the unit direction over r^2
    assert!(near(s.list[1].vel, Vec3::new(2.0, 0.0, 0.0)));
}

#[test]
fn random_actions_from_a_point_domain() {
    let one = [(Vec3::ZERO, X)];
    let two_up = pt(Vec3::new(0.0, 2.0, 0.0));
    let mut s = store(&one);
    step(ActionKind::RandomVelocity, ActionParams::Random { domain: two_up }, &mut s, 0.5);
    assert_eq!(s.list[0].vel, Vec3::new(0.0, 2.0, 0.0));
    let mut s = store(&one);
    step(ActionKind::RandomAccel, ActionParams::Random { domain: two_up }, &mut s, 0.5);
    assert_eq!(s.list[0].vel, Vec3::new(1.0, 1.0, 0.0));
    let mut s = store(&one);
    step(ActionKind::RandomDisplace, ActionParams::Random { domain: two_up }, &mut s, 0.5);
    assert_eq!(s.list[0].pos, Vec3::new(0.0, 1.0, 0.0));
}

#[test]
fn sinks_remove_by_place_and_by_velocity() {
    let mut s = store(&[(Vec3::ZERO, X), (Vec3::splat(5.0), X)]);
    let around_origin = FDomain { kind: dk::BOX, p1: Vec3::splat(-1.0), p2: Vec3::ONE, ..FDomain::default() };
    step(ActionKind::Sink, ActionParams::Sink { kill_inside: 1, domain: around_origin }, &mut s, 1.0);
    assert_eq!(s.list.len(), 1);
    assert_eq!(s.list[0].pos, Vec3::splat(5.0));
    let sphere = FDomain { kind: dk::SPHERE, p1: X, radius1: 0.5, radius1_sqr: 0.25, ..FDomain::default() };
    step(ActionKind::SinkVelocity, ActionParams::Sink { kill_inside: 1, domain: sphere }, &mut s, 1.0);
    assert!(s.list.is_empty());
}

#[test]
fn speed_rotation_and_velocity_targets() {
    let mut s = store(&[(Vec3::ZERO, X)]);
    step(ActionKind::SpeedLimit, ActionParams::SpeedLimit { min_speed: 2.0, max_speed: 9.0 }, &mut s, 1.0);
    assert!(near(s.list[0].vel, Vec3::new(2.0, 0.0, 0.0)));
    step(ActionKind::SpeedLimit, ActionParams::SpeedLimit { min_speed: 0.0, max_speed: 0.5 }, &mut s, 1.0);
    assert!(near(s.list[0].vel, Vec3::new(0.5, 0.0, 0.0)));
    let mut s = store(&[(Vec3::ZERO, X)]);
    step(ActionKind::TargetRotate, ActionParams::Target { value: X, scale: 1.0 }, &mut s, 0.5);
    assert!(near(s.list[0].rot, Vec3::new(0.5, 0.0, 0.0)));
    step(
        ActionKind::TargetVelocity,
        ActionParams::Target { value: Vec3::new(0.0, 0.0, 2.0), scale: 1.0 },
        &mut s,
        0.5,
    );
    assert!(near(s.list[0].vel, Vec3::new(0.5, 0.0, 1.0)));
}

#[test]
fn turbulence_keeps_the_speed() {
    let mut s = store(&[(Vec3::new(0.3, 0.7, 0.2), Vec3::new(0.0, 3.0, 4.0))]);
    step(
        ActionKind::Turbulence,
        ActionParams::Turbulence {
            frequency: 1.0,
            octaves: 2,
            magnitude: 5.0,
            epsilon: 0.1,
            offset: Vec3::ONE,
        },
        &mut s,
        0.1,
    );
    assert!((s.list[0].vel.length() - 5.0).abs() < 1e-4);
}

#[test]
fn avoid_bends_away_and_bounce_reflects() {
    let mut s = store(&[(Vec3::new(0.0, 1.2, 0.0), X)]);
    let sphere =
        FDomain { kind: dk::SPHERE, p1: Vec3::new(3.0, 1.0, 0.0), radius1: 1.0, ..FDomain::default() };
    step(
        ActionKind::Avoid,
        ActionParams::Avoid { position: sphere, look_ahead: 10.0, magnitude: 1.0, epsilon: 0.0 },
        &mut s,
        1.0,
    );
    // turned up, away from the sphere's centre, the speed kept
    assert!(s.list[0].vel.y > 0.0 && (s.list[0].vel.length() - 1.0).abs() < 1e-5);
    let mut s = store(&[(Vec3::new(0.0, 0.1, 0.0), Vec3::new(0.0, -1.0, 0.0))]);
    let floor = FDomain { kind: dk::PLANE, p1: Vec3::ZERO, p2: Vec3::Y, ..FDomain::default() };
    step(
        ActionKind::Bounce,
        ActionParams::Bounce { position: floor, one_minus_friction: 1.0, resilience: 0.5, cutoff_sqr: 0.0 },
        &mut s,
        1.0,
    );
    // the normal part reversed and halved, no tangent part
    assert!(near(s.list[0].vel, Vec3::new(0.0, 0.5, 0.0)));
}
