//! An ellipsoid of the given radius and height slid from
//! one point to another over the collision model - what decides whether a node can be walked to.
//! Float for float with the engine's own movement: the sphere-space triangles, the collision response that slides along
//! the nearest contact, the push out of a stuck position.

use glam::Vec3;

use super::collide::CollideModel;
use super::xr::{self, EPS, EPS_L, EPS_S, dot, fsimilar, mad, magnitude, normalize_safe, ray_tri2};
use crate::math::Aabb;

const COLLIDE_ACT_DEPTH: u16 = 8;
const COLLIDE_ACT_STUCK_DEPTH: i32 = 16;
const CL_EPSILON: f32 = EPS_L;

/// `cl_tri`: a triangle in the ellipsoid's unit space.
#[derive(Clone, Copy, Debug)]
struct ClTri {
    e10: Vec3,
    e10s: f32,
    e21: Vec3,
    e21s: f32,
    e02: Vec3,
    e02s: f32,
    p: [Vec3; 3],
    n: Vec3,
    d: f32,
}

/// The collision data of one move.
#[derive(Clone, Copy, Debug, Default)]
struct Collision {
    velocity: Vec3,
    source: Vec3,
    last_safe: Vec3,
    stuck: bool,
    found: bool,
    nearest_distance: f32,
    nearest_point: Vec3,
    nearest_poly_point: Vec3,
}

fn div(v: Vec3, s: f32) -> Vec3 {
    Vec3::new(v.x / s, v.y / s, v.z / s)
}

fn mul(v: Vec3, s: f32) -> Vec3 {
    Vec3::new(v.x * s, v.y * s, v.z * s)
}

/// The closest point on an edge.
fn closest_on_edge(a: Vec3, b: Vec3, ed: Vec3, elen: f32, p: Vec3) -> Vec3 {
    let c = p - a;
    let t = dot(ed, c);
    if t <= 0.0 {
        return a;
    }
    if t >= elen {
        return b;
    }
    mad(a, ed, t)
}

/// `closestPointOnTriangle(result, cl_tri, P)`.
fn closest_on_tri(t: &ClTri, p: Vec3) -> Vec3 {
    let rab = closest_on_edge(t.p[0], t.p[1], t.e10, t.e10s, p);
    let dab = xr::dist_sqr(p, rab);
    let rbc = closest_on_edge(t.p[1], t.p[2], t.e21, t.e21s, p);
    let dbc = xr::dist_sqr(p, rbc);
    let rca = closest_on_edge(t.p[2], t.p[0], t.e02, t.e02s, p);
    let dca = xr::dist_sqr(p, rca);
    let (min, mut result) = if dbc < dab { (dbc, rbc) } else { (dab, rab) };
    if dca < min {
        result = rca;
    }
    result
}

/// The distance to the sphere along the ray, -1 for a miss.
fn ray_sphere(ro: Vec3, rv: Vec3, so: Vec3, sr: f32) -> f32 {
    let q = so - ro;
    let c = magnitude(q);
    let v = dot(q, rv);
    let d = sr * sr - (c * c - v * v);
    if d < 0.0 {
        return -1.0;
    }
    v - d.sqrt()
}

/// Whether a point is inside a sphere.
fn in_sphere(p: Vec3, so: Vec3, sr: f32) -> bool {
    xr::dist_sqr(so, p) < sr * sr
}

/// The intersection of a ray with a plane.
fn ray_plane(ro: Vec3, rd: Vec3, po: Vec3, pn: Vec3) -> f32 {
    let numer = dot(ro - po, pn);
    let denom = dot(pn, rd);
    if denom == 0.0 {
        return -1.0;
    }
    -(numer / denom)
}

/// `msimulator_CheckCollision`.
fn check_collision(cl: &mut Collision, tris: &[ClTri]) {
    let source = cl.source;
    let velocity = cl.velocity;
    let nv = normalize_safe(cl.velocity);
    let distance_to_travel = magnitude(velocity);
    for t in tris {
        // backfaces are ignored (a NaN normal too)
        let facing = dot(t.n, nv) < 0.0;
        if !facing {
            continue;
        }
        let mut s_point = source - t.n;
        let (inside, dist_plane, p_point) = if dot(s_point, t.n) + t.d < -EPS_S {
            let (inside, d) = ray_tri2(s_point, t.n, &t.p);
            (inside, d, mad(s_point, t.n, d))
        } else {
            let (inside, d) = ray_tri2(s_point, nv, &t.p);
            (inside, d, mad(s_point, nv, d))
        };
        let mut poly_point = p_point;
        let mut dist_ellipsoid = dist_plane;
        if !inside {
            poly_point = closest_on_tri(t, p_point);
            let back = Vec3::new(-nv.x, -nv.y, -nv.z);
            dist_ellipsoid = ray_sphere(poly_point, back, source, 1.0);
            if dist_ellipsoid >= 0.0 {
                s_point = mad(poly_point, nv, -dist_ellipsoid);
            }
        }
        if in_sphere(poly_point, source, 1.0) {
            cl.stuck = true;
        }
        if dist_ellipsoid >= 0.0
            && dist_ellipsoid <= distance_to_travel
            && (!cl.found || dist_ellipsoid < cl.nearest_distance)
        {
            cl.nearest_distance = dist_ellipsoid;
            cl.nearest_point = s_point;
            cl.nearest_poly_point = poly_point;
            cl.found = true;
        }
    }
}

/// `msimulator_ResolveStuck`.
fn resolve_stuck(tris: &[ClTri], position: &mut Vec3) {
    let safe_r = 1.0 + EPS_L * 2.0;
    for _ in 0..COLLIDE_ACT_STUCK_DEPTH {
        let mut stuck_dir = Vec3::ZERO;
        let mut count = 0u32;
        for t in tris {
            let n_inv = Vec3::new(-t.n.x, -t.n.y, -t.n.z);
            let (inside, dist) = ray_tri2(*position, n_inv, &t.p);
            let poly = if inside {
                mad(*position, n_inv, dist)
            } else {
                closest_on_tri(t, mad(*position, n_inv, dist))
            };
            if in_sphere(poly, *position, safe_r) {
                let dir = *position - poly;
                let len = magnitude(dir);
                stuck_dir += mul(dir, (safe_r - len) / len);
                count += 1;
            }
        }
        if count == 0 {
            break;
        }
        stuck_dir = div(stuck_dir, count as f32);
        *position += stuck_dir;
        if magnitude(stuck_dir) < EPS {
            break;
        }
    }
}

/// `msimulator_CollideWithWorld`, its recursion as a loop.
fn collide_with_world(cl: &mut Collision, tris: &[ClTri], mut position: Vec3, mut velocity: Vec3) -> Vec3 {
    let mut cnt: u16 = 0;
    loop {
        resolve_stuck(tris, &mut position);
        if magnitude(velocity) < EPS_L {
            cl.velocity = Vec3::ZERO;
            return position;
        }
        if cnt > COLLIDE_ACT_DEPTH {
            return cl.last_safe;
        }
        let destination = position + velocity;
        cl.velocity = velocity;
        cl.source = position;
        cl.found = false;
        cl.stuck = false;
        cl.nearest_distance = -1.0;
        check_collision(cl, tris);
        if !cl.found && !cl.stuck {
            let l = magnitude(velocity);
            let v = mul(velocity, (l - CL_EPSILON) / l);
            let ret = position + v;
            cl.last_safe = ret;
            return ret;
        }
        if cl.stuck {
            return cl.last_safe;
        }
        let new_source = if cl.nearest_distance >= CL_EPSILON {
            // `set_length`
            let v = mul(velocity, (cl.nearest_distance - CL_EPSILON) / magnitude(velocity));
            cl.source + v
        } else {
            cl.source
        };
        let origin = cl.nearest_poly_point;
        let normal = new_source - cl.nearest_poly_point;
        let l = ray_plane(destination, normal, origin, normal);
        let new_destination = mad(destination, normal, l);
        let new_velocity = new_destination - cl.nearest_poly_point;
        cl.last_safe = position;
        position = new_source;
        velocity = new_velocity;
        cnt += 1;
    }
}

/// The buffers one move works in, kept between moves by a caller that makes many.
#[derive(Default)]
pub struct Scratch {
    found: Vec<u32>,
    tris: Vec<ClTri>,
}

/// Where an ellipsoid of `radius` and `height` standing at
/// `start` ends up when it is moved towards `end`.
#[must_use]
pub fn simulate(model: &CollideModel, start: Vec3, end: Vec3, radius: f32, height: f32) -> Vec3 {
    simulate_in(&mut Scratch::default(), model, start, end, radius, height)
}

/// [`simulate`] in the caller's buffers.
#[must_use]
pub fn simulate_in(
    scratch: &mut Scratch,
    model: &CollideModel,
    start: Vec3,
    end: Vec3,
    radius: f32,
    height: f32,
) -> Vec3 {
    let half_height = height / 2.0;
    let mk = |p: Vec3| {
        Aabb::new(
            Vec3::new(p.x - radius, p.y, p.z - radius),
            Vec3::new(p.x + radius, p.y + height, p.z + radius),
        )
    };
    let (b1, b2) = (mk(start), mk(end));
    let mut bb = Aabb::new(b1.min.min(b2.min), b1.max.max(b2.max));
    bb.grow(0.05);
    let Scratch { found, tris } = scratch;
    model.box_query_into(&bb, true, found, |_| true);

    let xf = Vec3::new(1.0 / radius, 1.0 / half_height, 1.0 / radius);
    let mut lposition = start;
    lposition.y += half_height;
    lposition *= xf;
    let mut target = end;
    target.y += half_height;
    target *= xf;
    let mut lvelocity = (end - start) * xf;

    let mut cl = Collision { last_safe: lposition, ..Collision::default() };
    tris.clear();
    tris.extend(found.iter().filter_map(|&i| model.tri(i)).map(|v| {
        let p = [v[0] * xf, v[1] * xf, v[2] * xf];
        let n = xr::mknormal(p[0], p[1], p[2]);
        let d = -dot(n, p[0]);
        let (e10, e21, e02) = (p[1] - p[0], p[2] - p[1], p[0] - p[2]);
        let (s10, s21, s02) = (magnitude(e10), magnitude(e21), magnitude(e02));
        ClTri {
            e10: div(e10, s10),
            e10s: s10,
            e21: div(e21, s21),
            e21s: s21,
            e02: div(e02, s02),
            e02s: s02,
            p,
            n,
            d,
        }
    }));

    let mut pos = lposition;
    for _ in 0..3 {
        pos = collide_with_world(&mut cl, tris, lposition, lvelocity);
        if fsimilar(pos.x, target.x, EPS) && fsimilar(pos.z, target.z, EPS) {
            break;
        }
        lposition = pos;
        lvelocity = target - pos;
        lvelocity.y = 0.0;
    }
    let mut result = Vec3::new(pos.x / xf.x, pos.y / xf.y, pos.z / xf.z);
    result.y -= half_height;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> Vec<[Vec3; 3]> {
        vec![[a, b, c], [a, c, d]]
    }

    #[test]
    fn on_a_flat_floor_the_body_gets_there() {
        let s = 20.0;
        let tris =
            quad(Vec3::new(-s, 0.0, -s), Vec3::new(-s, 0.0, s), Vec3::new(s, 0.0, s), Vec3::new(s, 0.0, -s));
        let n = tris.len();
        let m = CollideModel::from_tris(tris, vec![0; n], vec!["default".into()]);
        let end = Vec3::new(0.7, 0.0, 0.0);
        let r = simulate(&m, Vec3::ZERO, end, 0.7 / 2f32.sqrt(), 0.7);
        assert!((r.x - end.x).abs() < 0.1 && (r.z - end.z).abs() < 0.1, "{r:?}");
    }

    #[test]
    fn a_wall_stops_it() {
        let s = 20.0;
        let mut tris =
            quad(Vec3::new(-s, 0.0, -s), Vec3::new(-s, 0.0, s), Vec3::new(s, 0.0, s), Vec3::new(s, 0.0, -s));
        // a wall at x = 0.5 facing -x
        tris.extend(quad(
            Vec3::new(0.5, -1.0, -s),
            Vec3::new(0.5, -1.0, s),
            Vec3::new(0.5, 3.0, s),
            Vec3::new(0.5, 3.0, -s),
        ));
        let n = tris.len();
        let m = CollideModel::from_tris(tris, vec![0; n], vec!["default".into()]);
        let end = Vec3::new(0.7, 0.0, 0.0);
        let r = simulate(&m, Vec3::ZERO, end, 0.7 / 2f32.sqrt(), 2.0);
        assert!(r.x < 0.5, "{r:?}");
    }
}
