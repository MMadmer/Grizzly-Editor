//! `scene.audit`: what is wrong with where things stand.
//!
//! A scene written as text cannot be seen by its writer: an object a metre above the ground, a
//! crate inside a wall, a stalker placed off the AI map are all sound files. The audit reads the
//! geometry and says so, one warning a finding, at the object it is about. Every check is
//! bounded and reads the same snapshot, so the same scene gives the same list.

use gz_editor::CmdError;
use rustc_hash::{FxHashMap, FxHashSet};
use schemars::JsonSchema;
use serde::Deserialize;

use super::ai::Map as AiMap;
use super::fit::blocked;
use super::{Scene, ground, ids_named};
use crate::class::ObjClass;
use crate::math::{Aabb, Vec3};
use crate::model::{ObjData, ObjId, Object};
use crate::objects::PointKind;
use crate::pick::{RayFilter, RayKinds, ray_scene};

/// The objects one audit reads at most: past it the caller names objects or an area.
pub const MAX_OBJECTS: usize = 5000;
/// A gap under an object that is still "standing", metres.
pub const GAP: f32 = 0.25;
/// How far down a thing is looked for under an object before it has nothing under it, metres.
pub const DEEP: f32 = 1000.0;
/// How far from the AI map's nearest node a walker may stand, metres.
pub const OFF_MAP: f32 = 1.0;
/// A footprint this long on both sides is ground, not a thing that stands on it, metres.
const GROUND_SIDE: f32 = 20.0;

/// The checks: code, word, what it finds.
pub const CHECKS: [(&str, &str, &str); 11] = [
    ("S201", "floating", "an object more than 0.25 m above whatever is under it, touching nothing"),
    ("S202", "buried", "an object more than half under the ground"),
    ("S203", "overlap", "two objects standing in each other"),
    ("S204", "duplicate", "the same model twice at the same place"),
    ("S205", "unsupported", "an object with nothing under it"),
    ("S206", "off_ai_map", "a creature's spawn point off the AI map"),
    ("S207", "underground", "a spawn point under the ground"),
    ("S208", "way_off_ai_map", "a way point off the AI map"),
    ("S209", "light_underground", "a light under the ground"),
    ("S210", "not_authored", "an object the game's level does not have that is not marked authored"),
    ("S211", "terrain_border", "a terrain whose border stands more than 0.5 m off the ground around it"),
];

/// `scene.audit`'s parameters.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditParams {
    /// The objects to check. Left out with `area` left out too: the objects marked `authored`
    /// when the scene has any (a level of the game is the game's: only what the module adds is
    /// checked), else every object.
    #[serde(default)]
    pub objects: Option<Vec<String>>,
    /// Instead of `objects`: the objects standing in this ground rectangle, `[x0, z0, x1, z1]`.
    #[serde(default)]
    pub area: Option<[f32; 4]>,
    /// Only these checks, by code (`S201`) or word (`floating`, `buried`, `overlap`, `duplicate`,
    /// `unsupported`, `off_ai_map`, `underground`, `way_off_ai_map`, `light_underground`,
    /// `not_authored`, `terrain_border`). Left out: all of them.
    #[serde(default)]
    pub checks: Option<Vec<String>>,
    /// At most this many problems in the answer (200); `total` counts them all.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// One thing the audit found.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub code: &'static str,
    pub object: ObjId,
    pub text: String,
}

/// What an audit found and what it could not look at.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Audit {
    pub findings: Vec<Finding>,
    /// How many objects were read.
    pub checked: usize,
    /// Checks that did not run, each with its reason.
    pub skipped: Vec<String>,
}

fn wanted(p: &AuditParams) -> Result<FxHashSet<&'static str>, CmdError> {
    let Some(list) = &p.checks else { return Ok(CHECKS.iter().map(|c| c.0).collect()) };
    let mut out = FxHashSet::default();
    for word in list {
        let w = word.trim();
        let Some(c) = CHECKS.iter().find(|c| c.0.eq_ignore_ascii_case(w) || c.1.eq_ignore_ascii_case(w))
        else {
            let all: Vec<&str> = CHECKS.iter().map(|c| c.1).collect();
            return Err(CmdError::invalid(format!(
                "checks takes {} or their codes, not '{w}'",
                all.join(", ")
            )));
        };
        out.insert(c.0);
    }
    Ok(out)
}

/// The objects an audit reads.
fn subjects(sc: &Scene<'_>, p: &AuditParams) -> Result<Vec<ObjId>, CmdError> {
    let mut ids: Vec<ObjId> = match (&p.objects, p.area) {
        (Some(_), Some(_)) => {
            return Err(CmdError::invalid("objects and area both choose what is checked: give one"));
        }
        (Some(names), None) => ids_named(sc.state, Some(names))?.unwrap_or_default(),
        (None, Some(a)) => {
            if !a.iter().all(|v| v.is_finite()) {
                return Err(CmdError::invalid("area takes four numbers: x0, z0, x1, z1"));
            }
            let (x0, x1, z0, z1) = (a[0].min(a[2]), a[0].max(a[2]), a[1].min(a[3]), a[1].max(a[3]));
            sc.state
                .objects
                .values()
                .filter(|o| {
                    let at = o.position();
                    at.x >= x0 && at.x <= x1 && at.z >= z0 && at.z <= z1
                })
                .map(|o| o.id)
                .collect()
        }
        (None, None) => {
            let own: Vec<ObjId> =
                sc.state.objects.values().filter(|o| o.marks.authored).map(|o| o.id).collect();
            if own.is_empty() { sc.state.objects.keys().copied().collect() } else { own }
        }
    };
    ids.sort_unstable_by_key(|id| id.0);
    ids.dedup();
    if ids.len() > MAX_OBJECTS {
        return Err(CmdError::invalid(format!(
            "an audit reads at most {MAX_OBJECTS} objects, not {}: name objects or an area",
            ids.len()
        )));
    }
    Ok(ids)
}

fn name(sc: &Scene<'_>, id: ObjId) -> String {
    sc.state.get(id).map_or_else(|| "something".to_owned(), |o| o.name.to_string())
}

/// Whether a box is ground: long on both sides.
fn is_ground(b: &Aabb) -> bool {
    b.max.x - b.min.x >= GROUND_SIDE && b.max.z - b.min.z >= GROUND_SIDE
}

/// A solid object with its boxes: the world's and its model's own.
struct Solid<'a> {
    o: &'a Object,
    world: Aabb,
    local: Aabb,
}

fn solid<'a>(sc: &Scene<'a>, o: &'a Object) -> Option<Solid<'a>> {
    let ObjData::SceneObject(d) = &o.data else { return None };
    let local = sc.lib.cached(&d.reference)?.bounds;
    let world = sc.world_box(o)?;
    Some(Solid { o, world, local })
}

/// S201 and S205: what an object stands on, or that it stands on nothing.
fn standing(sc: &Scene<'_>, s: &Solid<'_>) -> Option<Finding> {
    let except = [s.o.id];
    let f = RayFilter { kinds: RayKinds::Meshes, only: None, except: &except };
    let b = &s.world;
    let c = b.center();
    // its middle and its four corners pulled in a tenth, from just above its bottom
    let (ix, iz) = ((b.max.x - b.min.x) * 0.4, (b.max.z - b.min.z) * 0.4);
    let feet = [(0.0, 0.0), (-ix, -iz), (ix, -iz), (ix, iz), (-ix, iz)];
    let mut nearest: Option<(f32, ObjId)> = None;
    for (dx, dz) in feet {
        let from = Vec3::new(c.x + dx, b.min.y + 0.05, c.z + dz);
        if let Some(h) = ground::down(sc, from, DEEP, &f) {
            let gap = b.min.y - h.point.y;
            if nearest.is_none_or(|(g, _)| gap < g) {
                nearest = Some((gap, h.id));
            }
        }
    }
    if nearest.is_some_and(|(gap, _)| gap <= GAP) {
        return None;
    }
    // hung from above, or held by what it touches: a lamp under a ceiling, a sign on a wall
    let top = Vec3::new(c.x, b.max.y - 0.05, c.z);
    if ray_scene(sc.pick, sc.state, sc.lib, top, Vec3::Y, GAP + 0.05, &f).is_some() {
        return None;
    }
    let scale = s.o.xform.scale.abs().max(Vec3::splat(1e-6));
    let grown = Aabb::new(s.local.min - Vec3::splat(0.1) / scale, s.local.max + Vec3::splat(0.1) / scale);
    if !blocked(sc, &grown, &s.o.xform, &f).blocked.is_empty() {
        return None;
    }
    Some(match nearest {
        Some((gap, under)) => Finding {
            code: "S201",
            object: s.o.id,
            text: format!("{} floats {gap:.2} m above {}: nothing holds it", s.o.name, name(sc, under)),
        },
        None => Finding {
            code: "S205",
            object: s.o.id,
            text: format!(
                "{} has nothing under it within {DEEP:.0} m: it would fall out of the level",
                s.o.name
            ),
        },
    })
}

/// S202: a floor passes through an object above its middle.
fn buried(sc: &Scene<'_>, s: &Solid<'_>) -> Option<Finding> {
    let except = [s.o.id];
    let f = RayFilter { kinds: RayKinds::Meshes, only: None, except: &except };
    let b = &s.world;
    let c = b.center();
    let h = ground::down(sc, Vec3::new(c.x, b.max.y - 0.01, c.z), b.max.y - b.min.y, &f)?;
    let floor = h.normal.is_some_and(|n| n.y.abs() > 0.7);
    (floor && h.point.y > c.y + 0.05).then(|| Finding {
        code: "S202",
        object: s.o.id,
        text: format!(
            "{} is more than half under {}: its surface passes {:.2} m above the object's bottom",
            s.o.name,
            name(sc, h.id),
            h.point.y - b.min.y
        ),
    })
}

/// S203: the objects this one stands in, each pair once. Two bodies overlap when the triangles
/// of one reach into the other's box and their boxes share more than a touch: how deep is the
/// least their boxes share along an axis (models are shells, so the triangles alone do not say
/// on which side the body is).
fn overlaps(sc: &Scene<'_>, s: &Solid<'_>, seen: &mut FxHashSet<(u32, u32)>, out: &mut Vec<Finding>) {
    let except = [s.o.id];
    let f = RayFilter { kinds: RayKinds::Meshes, only: None, except: &except };
    let size = |b: &Aabb| (b.max - b.min).min_element().max(0.01);
    for (id, _) in blocked(sc, &s.local, &s.o.xform, &f).blocked {
        let Some(other) = sc.state.get(id).and_then(|o| sc.world_box(o)) else { continue };
        if is_ground(&other) {
            continue;
        }
        let shared = (s.world.max.min(other.max) - s.world.min.max(other.min)).min_element();
        let need = (0.3 * size(&s.world).min(size(&other))).max(0.1);
        let pair = (s.o.id.0.min(id.0), s.o.id.0.max(id.0));
        if shared > need && seen.insert(pair) {
            out.push(Finding {
                code: "S203",
                object: s.o.id,
                text: format!("{} stands {shared:.2} m inside {}", s.o.name, name(sc, id)),
            });
        }
    }
}

/// How far a terrain's border may stand off the ground around it, metres.
pub const BORDER: f32 = 0.5;

/// S211: a terrain's border against the ground just outside it, at eight places a side.
fn terrain_border(sc: &Scene<'_>, o: &Object) -> Option<Finding> {
    let t = crate::objects::terrain::terrain(o)?;
    let (hi, lo) = sc.top()?;
    let own = [o.id];
    let on = RayFilter { kinds: RayKinds::Meshes, only: Some(&own), except: &[] };
    let off = RayFilter { kinds: RayKinds::Meshes, only: None, except: &own };
    let half = t.half();
    let mut worst = 0.0f32;
    for side in 0..4 {
        for n in 0..8 {
            // along the side, clear of the corners
            let u = -0.9 + 1.8 * n as f32 / 7.0;
            // a little inside the border (off the skirt) and a little outside it
            let at = |d: f32| match side {
                0 => glam::Vec2::new(u * half.x, half.y + d),
                1 => glam::Vec2::new(u * half.x, -half.y - d),
                2 => glam::Vec2::new(half.x + d, u * half.y),
                _ => glam::Vec2::new(-half.x - d, u * half.y),
            };
            let inside = ground::down(sc, t.to_world(at(-0.1), hi), hi - lo, &on);
            let outside = ground::down(sc, t.to_world(at(0.4), hi), hi - lo, &off);
            if let (Some(a), Some(b)) = (inside, outside) {
                worst = worst.max((a.point.y - b.point.y).abs());
            }
        }
    }
    (worst > BORDER).then(|| Finding {
        code: "S211",
        object: o.id,
        text: format!("{}'s border stands {worst:.2} m off the ground around it: a step shows there", o.name),
    })
}

/// A model, where it stands, how it is turned and how large it is: centimetres, tenths of a
/// degree, hundredths.
type Placement = (String, [i64; 3], [i64; 3], [i64; 3]);

/// The key two placements of one model share when they are one placement twice.
fn placement(o: &Object) -> Option<Placement> {
    let ObjData::SceneObject(d) = &o.data else { return None };
    let q = |v: Vec3, k: f32| [(v.x * k).round() as i64, (v.y * k).round() as i64, (v.z * k).round() as i64];
    Some((
        d.reference.to_lowercase(),
        q(o.xform.position, 100.0),
        // a tenth of a degree
        q(o.xform.rotation, 180.0 / std::f32::consts::PI * 10.0),
        q(o.xform.scale, 100.0),
    ))
}

/// Whether nothing is under a point within 50 m while something is above it: under the ground.
fn under_the_ground(sc: &Scene<'_>, at: Vec3) -> Option<ObjId> {
    let f = RayFilter { kinds: RayKinds::Meshes, only: None, except: &[] };
    let (hi, _) = sc.top()?;
    if hi <= at.y + 0.1 {
        return None;
    }
    let above =
        ray_scene(sc.pick, sc.state, sc.lib, Vec3::new(at.x, hi, at.z), Vec3::NEG_Y, hi - at.y - 0.05, &f)?;
    ground::down(sc, at + Vec3::Y * 0.05, 50.0, &f).is_none().then_some(above.id)
}

/// Whether a spawn point's entity walks on the AI map.
fn walks(o: &Object) -> bool {
    let ObjData::Spawn(d) = &o.data else { return false };
    if d.kind == PointKind::PlayerStart {
        return true;
    }
    d.entity
        .as_ref()
        .and_then(crate::objects::SpawnEntity::entity)
        .and_then(gz_entities::Entity::class_info)
        .is_some_and(|c| {
            let id = c.clsid.trim();
            // stalkers and everything derived from a creature but what flies
            id == "AI_STL_S" || (c.class.is_creature() && id != "AI_CROW")
        })
}

/// Runs the checks. `base` is the names of the objects the game's own level has, lower case,
/// when they are known: without them `S210` does not run. Blocking: call it on the CPU pool.
pub fn audit(sc: &Scene<'_>, p: &AuditParams, base: Option<&FxHashSet<String>>) -> Result<Audit, CmdError> {
    let want = wanted(p)?;
    let ids = subjects(sc, p)?;
    let mut out = Audit { checked: ids.len(), ..Audit::default() };
    let objects: Vec<&Object> = ids.iter().filter_map(|id| sc.state.get(*id).map(|o| &**o)).collect();

    // duplicates are looked for among every placement of the scene
    let mut placed: FxHashMap<Placement, ObjId> = FxHashMap::default();
    if want.contains("S204") {
        let mut all: Vec<&Object> = sc.state.of_class(ObjClass::SceneObject).map(|o| &**o).collect();
        all.sort_unstable_by_key(|o| o.id.0);
        for o in all {
            if let Some(k) = placement(o) {
                placed.entry(k).or_insert(o.id);
            }
        }
    }
    let ai = sc.state.bulk.ai_map.as_ref().map(|m| AiMap::new(m.all_nodes(), m.header.params.patch_size));
    if ai.is_none() && (want.contains("S206") || want.contains("S208")) && p.checks.is_some() {
        out.skipped.push("S206, S208: the scene has no AI map".to_owned());
    }
    if base.is_none() && want.contains("S210") && p.checks.is_some() {
        out.skipped.push("S210: the game's own level was not found among the base scenes".to_owned());
    }

    let mut seen: FxHashSet<(u32, u32)> = FxHashSet::default();
    for o in objects {
        if !o.flags.visible {
            continue;
        }
        let f = &mut out.findings;
        if let Some(s) = solid(sc, o) {
            if !is_ground(&s.world) {
                if (want.contains("S201") || want.contains("S205"))
                    && let Some(found) = standing(sc, &s)
                    && want.contains(found.code)
                {
                    f.push(found);
                }
                if want.contains("S202") {
                    f.extend(buried(sc, &s));
                }
                if want.contains("S203") {
                    overlaps(sc, &s, &mut seen, f);
                }
            }
            if want.contains("S204")
                && let Some(first) = placement(o).and_then(|k| placed.get(&k)).copied()
                && first != o.id
            {
                f.push(Finding {
                    code: "S204",
                    object: o.id,
                    text: format!(
                        "{} is {} once more: the same model at the same place",
                        o.name,
                        name(sc, first)
                    ),
                });
            }
        }
        match &o.data {
            ObjData::Spawn(_) => {
                let at = o.position();
                if want.contains("S207")
                    && let Some(above) = under_the_ground(sc, at)
                {
                    f.push(Finding {
                        code: "S207",
                        object: o.id,
                        text: format!(
                            "{} is under {}: nothing is under it within 50 m",
                            o.name,
                            name(sc, above)
                        ),
                    });
                } else if want.contains("S206")
                    && walks(o)
                    && let Some(map) = &ai
                    && let Some((_, d)) = map.nearest(at)
                    && d > OFF_MAP
                {
                    f.push(Finding {
                        code: "S206",
                        object: o.id,
                        text: format!("{} stands {d:.1} m off the AI map: it cannot walk from there", o.name),
                    });
                }
            }
            ObjData::Terrain(_) => {
                if want.contains("S211") {
                    f.extend(terrain_border(sc, o));
                }
            }
            ObjData::Light(_) => {
                if want.contains("S209")
                    && let Some(above) = under_the_ground(sc, o.position())
                {
                    f.push(Finding {
                        code: "S209",
                        object: o.id,
                        text: format!("{} is under {}: it lights nothing", o.name, name(sc, above)),
                    });
                }
            }
            ObjData::Way(w) => {
                if want.contains("S208")
                    && let Some(map) = &ai
                {
                    let off: Vec<String> = w
                        .points
                        .iter()
                        .filter(|wp| map.nearest(wp.position).is_none_or(|(_, d)| d > OFF_MAP))
                        .map(|wp| wp.name.to_string())
                        .collect();
                    if !off.is_empty() {
                        let shown: Vec<&str> = off.iter().take(5).map(String::as_str).collect();
                        let more = if off.len() > 5 {
                            format!(" and {} more", off.len() - 5)
                        } else {
                            String::new()
                        };
                        f.push(Finding {
                            code: "S208",
                            object: o.id,
                            text: format!("{}: {}{more} off the AI map", o.name, shown.join(", ")),
                        });
                    }
                }
            }
            _ => {}
        }
        if want.contains("S210")
            && let Some(base) = base
            && !o.marks.authored
            && !matches!(o.class, ObjClass::Sector | ObjClass::Portal | ObjClass::Group)
            && o.owner.is_none()
            && !base.contains(&o.name.to_lowercase())
        {
            f.push(Finding {
                code: "S210",
                object: o.id,
                text: format!(
                    "{} is not in the game's level and is not marked authored: it will not ship",
                    o.name
                ),
            });
        }
    }
    Ok(out)
}
