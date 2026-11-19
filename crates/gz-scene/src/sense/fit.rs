//! `fit` and `free`: whether a model fits somewhere without meeting anything, and where near a
//! point it does, standing on the ground.

use gz_editor::CmdError;
use serde_json::{Value, json};

use super::ground::down;
use super::rays::{OBox, object_meets};
use super::{
    Lists, MAX_RADIUS, MAX_SPOTS, QueryParams, Scene, facing, finite, is_solid, name_of, opt_r4, radians,
    slope_of,
};
use crate::cmd::{r4, v3};
use crate::landing::DROP_REACH;
use crate::math::{Aabb, Transform, Vec3};
use crate::model::{ObjData, ObjId};
use crate::pick::{RayFilter, RayKinds};

/// How far another object may touch the model without blocking it, metres: what stands on the
/// ground touches it.
const MARGIN: f32 = 0.02;

/// The model a fit or free is about: its box in its own space, where it stands, and the scene
/// object it is (which then does not count).
struct Model {
    local: Aabb,
    xform: Transform,
    itself: Option<ObjId>,
}

fn model_of(sc: &Scene<'_>, p: &QueryParams, what: &str) -> Result<Model, CmdError> {
    let given = [p.r#ref.is_some(), p.size.is_some(), p.object.is_some()].iter().filter(|x| **x).count();
    if given != 1 {
        let also = if what == "fit" { ", or object: a scene object" } else { "" };
        return Err(CmdError::invalid(format!(
            "{what} takes one of ref (a library model) or size: [x, y, z]{also}"
        )));
    }
    if let Some(r) = p.r#ref.as_deref() {
        let lo = sc
            .lib
            .cached(r.trim())
            .ok_or_else(|| CmdError::not_found(format!("there is no library object {}", r.trim())))?;
        return Ok(Model { local: lo.bounds, xform: Transform::default(), itself: None });
    }
    if let Some(s) = p.size {
        let s = finite(s, "size")?;
        if s.cmple(Vec3::ZERO).any() {
            return Err(CmdError::invalid("size takes numbers above 0"));
        }
        let local = Aabb::new(Vec3::new(-s.x * 0.5, 0.0, -s.z * 0.5), Vec3::new(s.x * 0.5, s.y, s.z * 0.5));
        return Ok(Model { local, xform: Transform::default(), itself: None });
    }
    let name = p.object.as_deref().unwrap_or_default().trim();
    let o =
        sc.state.find_any(name).ok_or_else(|| CmdError::not_found(format!("no object is named {name}")))?;
    // a spawn point stands there as the model its entity wears
    if let Some(r) = crate::objects::visual_ref(o) {
        let lo = sc
            .lib
            .cached(&r)
            .ok_or_else(|| CmdError::not_found(format!("the model of {name} ({r}) could not be read")))?;
        return Ok(Model { local: lo.bounds, xform: o.xform, itself: Some(o.id) });
    }
    let ObjData::SceneObject(d) = &o.data else {
        return Err(CmdError::invalid(format!("{name} is no object with a model")));
    };
    let lo = sc.lib.cached(&d.reference).ok_or_else(|| {
        CmdError::not_found(format!("the model of {name} ({}) could not be read", d.reference))
    })?;
    Ok(Model { local: lo.bounds, xform: o.xform, itself: Some(o.id) })
}

/// What of a triangle lies inside a box (Sutherland-Hodgman against its six planes).
fn clip_to_box(t: &[Vec3; 3], lo: Vec3, hi: Vec3) -> Vec<Vec3> {
    let mut poly: Vec<Vec3> = t.to_vec();
    for axis in 0..3 {
        for (bound, keep_above) in [(lo[axis], true), (hi[axis], false)] {
            if poly.is_empty() {
                return poly;
            }
            let side = |p: &Vec3| if keep_above { p[axis] - bound } else { bound - p[axis] };
            let mut next = Vec::with_capacity(poly.len() + 2);
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                let (da, db) = (side(&a), side(&b));
                if da >= 0.0 {
                    next.push(a);
                }
                if (da >= 0.0) != (db >= 0.0) {
                    next.push(a.lerp(b, da / (da - db)));
                }
            }
            poly = next;
        }
    }
    poly
}

/// The ground under a model's footprint.
#[derive(Clone, Copy, Debug)]
struct Under {
    min: f32,
    max: f32,
    centre: Option<f32>,
    normal: Option<Vec3>,
    /// Every sample is found and lies within [`EVEN`] of the plane under the centre.
    even: bool,
}

/// How far the ground under a footprint may stand off the plane under its centre and still be
/// one floor, metres.
const EVEN: f32 = 0.1;

/// What stands in a model's way at a place.
pub(super) struct Placed {
    pub(super) blocked: Vec<(ObjId, f32)>,
    bottom: f32,
}

/// How far above the point a search for a free place starts, metres: the floor a person at the
/// point stands on is found, not the storey above.
const HEAD: f32 = 1.5;

fn filter<'a>(lists: &'a Lists, except: &'a [ObjId]) -> RayFilter<'a> {
    // what a model meets is what stops a body
    RayFilter { kinds: RayKinds::Solid, only: lists.only.as_deref(), except }
}

/// The solid objects a model at `x` meets, each with how deep, metres.
pub(super) fn blocked(sc: &Scene<'_>, local: &Aabb, x: &Transform, f: &RayFilter<'_>) -> Placed {
    let ob = OBox::new(local, x.matrix());
    let world = ob.world();
    let scale = x.scale.abs().max(Vec3::splat(1e-6));
    let margin = Vec3::splat(MARGIN) / scale;
    let mut out = Vec::new();
    for id in sc.pick.box_candidates(&world) {
        let Some(o) = sc.state.get(id) else { continue };
        if !is_solid(o) || !f.accepts(o) {
            continue;
        }
        let tris = object_meets(sc, id, &ob, margin);
        if tris.is_empty() {
            continue;
        }
        // the depth: the shortest move along one of the model's axes that takes it off every
        // part of the triangles inside it
        let (lo, hi) = (ob.center - ob.half, ob.center + ob.half);
        let (inner_lo, inner_hi) = (lo + margin, hi - margin);
        let mut inside = Aabb::INVALID;
        for t in &tris {
            for q in clip_to_box(t, inner_lo, inner_hi) {
                inside.modify(q);
            }
        }
        if !inside.is_valid() {
            continue;
        }
        let moves = [inside.max - lo, hi - inside.min];
        let depth = moves
            .iter()
            .flat_map(|m| [m.x * scale.x, m.y * scale.y, m.z * scale.z])
            .fold(f32::MAX, f32::min)
            .max(0.0);
        out.push((id, depth));
    }
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    Placed { blocked: out, bottom: world.min.y }
}

/// The ground under nine points of a model's footprint (its corners, the middles of its sides
/// and its centre), rays starting at `from`.
fn under(sc: &Scene<'_>, local: &Aabb, x: &Transform, from: f32, f: &RayFilter<'_>) -> Option<Under> {
    let m = x.matrix();
    let c = local.center();
    let mut found: Vec<Vec3> = Vec::new();
    let (mut centre, mut normal) = (None, None);
    let mut mid = None;
    for (i, lx) in [local.min.x, c.x, local.max.x].into_iter().enumerate() {
        for (k, lz) in [local.min.z, c.z, local.max.z].into_iter().enumerate() {
            let w = m.transform_tiny(Vec3::new(lx, local.min.y, lz));
            let Some(h) = down(sc, Vec3::new(w.x, from, w.z), from - w.y + DROP_REACH, f) else { continue };
            found.push(h.point);
            if i == 1 && k == 1 {
                centre = Some(h.point.y);
                mid = Some(h.point);
                normal = h.normal.map(|n| facing(n, Vec3::NEG_Y));
            }
        }
    }
    let min = found.iter().map(|q| q.y).reduce(f32::min)?;
    let max = found.iter().map(|q| q.y).reduce(f32::max)?;
    let even = found.len() == 9
        && matches!((mid, normal), (Some(c), Some(n)) if found.iter().all(|q| n.dot(*q - c).abs() <= EVEN));
    Some(Under { min, max, centre, normal, even })
}

pub(super) fn fit(sc: &Scene<'_>, p: &QueryParams) -> Result<Value, CmdError> {
    let mut model = model_of(sc, p, "fit")?;
    match (p.position, model.itself) {
        (Some(at), _) => model.xform.position = finite(at, "position")?,
        (None, None) => return Err(CmdError::invalid("fit takes position: where the model's pivot goes")),
        (None, Some(_)) => {}
    }
    if let Some(r) = p.rotation {
        finite(r, "rotation")?;
        model.xform.rotation = radians(r);
    }
    if let Some(s) = p.scale {
        let s = finite(s, "scale")?;
        if s.abs().cmplt(Vec3::splat(1e-6)).any() {
            return Err(CmdError::invalid("a scale of zero"));
        }
        model.xform.scale = s;
    }
    let lists = Lists::of(sc.state, p)?;
    let mut except = lists.except.clone();
    except.extend(model.itself);
    let f = filter(&lists, &except);
    let placed = blocked(sc, &model.local, &model.xform, &f);
    let top = OBox::new(&model.local, model.xform.matrix()).world().max.y + 0.01;
    let ground = under(sc, &model.local, &model.xform, top, &f);
    let blocked_by: Vec<Value> = placed
        .blocked
        .iter()
        .map(|(id, depth)| json!({ "object": name_of(sc, *id), "depth": r4(*depth) }))
        .collect();
    let pos = model.xform.position;
    Ok(json!({
        "fits": placed.blocked.is_empty(),
        "blocked_by": blocked_by,
        "ground": ground.map_or(Value::Null, |g| json!({ "min": r4(g.min), "max": r4(g.max), "centre": opt_r4(g.centre) })),
        "gap": opt_r4(ground.and_then(|g| g.centre).map(|c| placed.bottom - c)),
        "rest": ground.map_or(Value::Null, |g| v3(Vec3::new(pos.x, pos.y + g.max - placed.bottom, pos.z))),
        "tilt": opt_r4(ground.and_then(|g| g.normal).map(slope_of)),
    }))
}

pub(super) fn free(sc: &Scene<'_>, p: &QueryParams) -> Result<Value, CmdError> {
    if p.object.is_some() {
        return Err(CmdError::invalid("free takes ref or size, not object"));
    }
    let model = model_of(sc, p, "free")?;
    let near =
        finite(p.position.ok_or_else(|| CmdError::invalid("free takes position: near where"))?, "position")?;
    let radius = p.radius.unwrap_or(10.0);
    if !radius.is_finite() || radius <= 0.0 || radius > MAX_RADIUS {
        return Err(CmdError::invalid(format!("radius takes metres above 0 and at most {MAX_RADIUS}")));
    }
    let count = p.count.unwrap_or(4);
    if count == 0 || count > MAX_SPOTS {
        return Err(CmdError::invalid(format!("count takes 1 to {MAX_SPOTS} spots")));
    }
    let size = model.local.max - model.local.min;
    let spacing = p.spacing.unwrap_or(size.x.max(size.z)).max(0.0);
    let slope = p.slope.unwrap_or(30.0);
    if !spacing.is_finite() || !slope.is_finite() {
        return Err(CmdError::invalid("spacing and slope take finite numbers"));
    }
    let lists = Lists::of(sc.state, p)?;
    let f = filter(&lists, &lists.except);
    // from just above the point: the floor of the storey it is on. `top` asks from above
    // everything instead (the highest floor: a roof, open ground)
    let Some((highest, _)) = sc.top() else { return Ok(json!({ "spots": [] })) };
    let from = if p.top { highest } else { (near.y + HEAD).min(highest) };
    // a grid around the point, nearest first; at most about 160 steps across
    let step = (spacing * 0.5).clamp(0.25, 2.0).max(2.0 * radius / 160.0);
    let n = (radius / step).ceil() as i32;
    let mut cands: Vec<(f32, Vec3)> = Vec::new();
    for i in -n..=n {
        for k in -n..=n {
            let off = Vec3::new(i as f32 * step, 0.0, k as f32 * step);
            let d = off.length();
            if d <= radius {
                cands.push((d, near + off));
            }
        }
    }
    cands.sort_by(|a, b| a.0.total_cmp(&b.0));
    let at_rest = |at: Vec3| -> Option<Vec3> {
        let x = Transform::at(at);
        let g = under(sc, &model.local, &x, from, &f)?;
        if !g.even || g.normal.is_some_and(|n| slope_of(n) > slope) {
            return None;
        }
        let bottom = OBox::new(&model.local, x.matrix()).world().min.y;
        let rest = Vec3::new(at.x, at.y + g.max - bottom, at.z);
        blocked(sc, &model.local, &Transform::at(rest), &f).blocked.is_empty().then_some(rest)
    };
    let mut spots: Vec<(Vec3, f32)> = Vec::new();
    for chunk in cands.chunks(256) {
        use rayon::prelude::*;
        let found: Vec<Option<Vec3>> = chunk.par_iter().map(|(_, at)| at_rest(*at)).collect();
        for ((d, _), rest) in chunk.iter().zip(found) {
            let Some(rest) = rest else { continue };
            let apart = |q: &Vec3| Vec3::new(q.x - rest.x, 0.0, q.z - rest.z).length() >= spacing;
            if spots.iter().all(|(q, _)| apart(q)) {
                spots.push((rest, *d));
            }
            if spots.len() >= count {
                break;
            }
        }
        if spots.len() >= count {
            break;
        }
    }
    let spots: Vec<Value> =
        spots.iter().map(|(at, d)| json!({ "position": v3(*at), "distance": r4(*d) })).collect();
    // the height the floors were looked for under: ground that rises over it within the radius
    // (a slope, the next storey) is not offered, and whoever asked can see why
    Ok(json!({ "spots": spots, "below": r4(from) }))
}
